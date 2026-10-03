//! Preview transcoding to HLS (fMP4 segments) and the player page.
//!
//! ffmpeg writes an EVENT playlist into the cache while it transcodes, so
//! playback can start as soon as the first segment exists, and the portion
//! transcoded so far is seekable. A `done` marker records completion; after
//! that the cached preview is served directly, even across restarts.

use std::{
    collections::HashSet,
    fs::Metadata,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{Arc, Mutex},
};

use anyhow::Context;
use axum::{
    extract::{self, Request, State},
    http::{HeaderValue, header},
    response::{Html, IntoResponse, Response},
};
use serde::Serialize;
use tokio::sync::{Notify, Semaphore};
use tower::ServiceExt;
use tower_http::services::ServeFile;

use crate::{
    AppError, AppState, PreviewConfig, assets, cache,
    html::{escape, human_size, page},
    media::{cache_key, child_command, ffmpeg_input, probe_duration},
};

/// Target segment length in seconds.
const SEGMENT_SECS: u32 = 2;

#[derive(Clone, Debug)]
pub enum JobState {
    Queued,
    Running,
    Done,
    Failed(String),
}

pub struct Job {
    src: PathBuf,
    dir: PathBuf,
    duration: Mutex<Option<f64>>,
    state: Mutex<JobState>,
}

fn hls_params(cfg: &PreviewConfig) -> String {
    format!(
        "hls-v2 w{} f{} crf{} seg{SEGMENT_SECS}",
        cfg.width, cfg.fps_max, cfg.crf
    )
}

fn job_dir(st: &AppState, key: &str) -> PathBuf {
    st.cache.join("hls").join(key)
}

/// Keys of transcodes that are queued or running, which must not be evicted.
pub fn active_keys(st: &AppState) -> HashSet<String> {
    st.jobs
        .lock()
        .unwrap()
        .iter()
        .filter(|(_, j)| {
            matches!(
                *j.state.lock().unwrap(),
                JobState::Queued | JobState::Running
            )
        })
        .map(|(k, _)| k.clone())
        .collect()
}

fn valid_key(key: &str) -> bool {
    key.len() == 32 && key.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Start a transcode for `key` unless one is running or already complete.
fn ensure_job(st: &AppState, key: &str, src: &Path) -> anyhow::Result<()> {
    let dir = job_dir(st, key);
    if dir.join("done").exists() {
        cache::touch(&dir);
        return Ok(());
    }
    let mut jobs = st.jobs.lock().unwrap();
    if let Some(j) = jobs.get(key)
        && matches!(
            *j.state.lock().unwrap(),
            JobState::Queued | JobState::Running
        )
    {
        return Ok(());
    }
    // Anything left here is a partial result from an interrupted or failed run.
    if dir.exists() {
        std::fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
    }
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let job = Arc::new(Job {
        src: src.to_owned(),
        dir,
        duration: Mutex::new(None),
        state: Mutex::new(JobState::Queued),
    });
    jobs.insert(key.to_owned(), job.clone());
    tokio::spawn(run_job(
        job,
        st.transcode_sem.clone(),
        st.cfg.clone(),
        st.cache_sweep.clone(),
    ));
    Ok(())
}

async fn run_job(job: Arc<Job>, sem: Arc<Semaphore>, cfg: PreviewConfig, sweep: Arc<Notify>) {
    *job.duration.lock().unwrap() = probe_duration(&job.src).await;
    let Ok(_permit) = sem.acquire_owned().await else {
        return;
    };
    *job.state.lock().unwrap() = JobState::Running;
    tracing::info!("transcoding {}", job.src.display());
    let t0 = std::time::Instant::now();
    let new_state = match transcode(&job, &cfg).await {
        Ok(()) => {
            tracing::info!("finished {} in {:.1?}", job.src.display(), t0.elapsed());
            JobState::Done
        }
        Err(e) => {
            tracing::warn!("transcode of {} failed: {e:#}", job.src.display());
            JobState::Failed(format!("{e:#}"))
        }
    };
    *job.state.lock().unwrap() = new_state;
    sweep.notify_one();
}

async fn transcode(job: &Job, cfg: &PreviewConfig) -> anyhow::Result<()> {
    let log_path = job.dir.join("ffmpeg.log");
    let log = std::fs::File::create(&log_path)?;
    let status = child_command("ffmpeg")
        .current_dir(&job.dir)
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y"])
        .arg("-i")
        .arg(ffmpeg_input(&job.src))
        .args(["-map", "0:v:0", "-map", "0:a:0?", "-sn", "-dn"])
        .arg("-vf")
        .arg(format!(
            "scale=w='trunc(min({},iw)/2)*2':h=-2:out_range=tv,format=yuv420p",
            cfg.width
        ))
        .arg("-fpsmax")
        .arg(cfg.fps_max.to_string())
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "veryfast",
            "-profile:v",
            "high",
        ])
        .arg("-crf")
        .arg(cfg.crf.to_string())
        .arg("-force_key_frames")
        .arg(format!("expr:gte(t,n_forced*{SEGMENT_SECS})"))
        .args(["-c:a", "aac", "-b:a", "96k", "-ac", "2"])
        .args(["-f", "hls", "-hls_time"])
        .arg(SEGMENT_SECS.to_string())
        .args([
            "-hls_playlist_type",
            "event",
            "-hls_segment_type",
            "fmp4",
            "-hls_fmp4_init_filename",
            "init.mp4",
            "-hls_segment_filename",
            "seg%05d.m4s",
            "-hls_flags",
            "temp_file+independent_segments",
            "index.m3u8",
        ])
        .stdout(Stdio::null())
        .stderr(log)
        .status()
        .await
        .context("running ffmpeg")?;
    if !status.success() {
        let log = tokio::fs::read_to_string(&log_path)
            .await
            .unwrap_or_default();
        let tail: Vec<&str> = log.lines().rev().take(5).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        anyhow::bail!("ffmpeg {status}: {}", tail.join("\n"));
    }
    tokio::fs::write(job.dir.join("done"), b"").await?;
    Ok(())
}

pub async fn play_page(
    st: &AppState,
    src: &Path,
    meta: &Metadata,
    uri_path: &str,
) -> Result<Response, AppError> {
    let key = cache_key(&hls_params(&st.cfg), "", src, meta);
    ensure_job(st, &key, src)?;

    let name = src.file_name().unwrap_or_default().to_string_lossy();
    let raw_href = uri_path.rsplit('/').next().unwrap_or("");
    let body = format!(
        r#"<p><a href="./">⬑ Back to directory</a> · <a href="{raw_href}">original</a> ({size})</p>
<h1>{name}</h1>
<video id="v" controls playsinline preload="auto"></video>
<p id="status">Starting preview…</p>
<script>
const key = "{key}";
const v = document.getElementById("v");
const status = document.getElementById("status");
let started = false;
function fmt(s) {{ return s == null ? "?" : s.toFixed(0) + " s"; }}
function play() {{
  v.play().catch(() => {{ v.muted = true; v.play().catch(() => {{}}); }});
}}
// Safari's native HLS is excellent; Chrome's native HLS mishandles growing
// EVENT playlists (no duration, seeks ignored), so everything else uses
// hls.js over MSE, falling back to native HLS only when MSE is missing.
// ?player=hlsjs or ?player=native forces one or the other.
const forced = new URLSearchParams(location.search).get("player");
const canNative = v.canPlayType("application/vnd.apple.mpegurl") !== "";
const isSafari = /Safari\//.test(navigator.userAgent) &&
  !/Chrome|Chromium|CriOS|FxiOS|Edg|Android/.test(navigator.userAgent);
let useNative = forced === "native" || (forced !== "hlsjs" && canNative && isSafari);
function loadHlsJs() {{
  return new Promise((resolve, reject) => {{
    const el = document.createElement("script");
    el.src = "{hls_js_url}";
    el.onload = resolve;
    el.onerror = () => reject(new Error("could not load hls.js"));
    document.head.appendChild(el);
  }});
}}
function attach() {{
  const url = "/_dir2web/hls/" + key + "/index.m3u8";
  if (useNative) {{ v.src = url; play(); return; }}
  const hls = new Hls({{ startPosition: 0 }});
  let recovered = false;
  hls.on(Hls.Events.ERROR, (_, d) => {{
    if (!d.fatal) return;
    if (d.type === Hls.ErrorTypes.NETWORK_ERROR) {{ hls.startLoad(); }}
    else if (d.type === Hls.ErrorTypes.MEDIA_ERROR && !recovered) {{ recovered = true; hls.recoverMediaError(); }}
    else {{ status.textContent = "Playback error: " + d.details; hls.destroy(); }}
  }});
  hls.on(Hls.Events.MANIFEST_PARSED, play);
  hls.loadSource(url);
  hls.attachMedia(v);
}}
async function poll() {{
  let s;
  try {{ s = await (await fetch("/_dir2web/status/" + key, {{cache: "no-store"}})).json(); }}
  catch (e) {{ status.textContent = "Status error: " + e; setTimeout(poll, 2000); return; }}
  if (!started && s.segments > 0) {{
    started = true;
    attach();
  }}
  if (s.state === "failed") {{ status.textContent = "Preview failed: " + s.error; return; }}
  if (s.state === "done") {{ status.textContent = "Preview complete (" + fmt(s.seconds) + ")."; return; }}
  if (s.state === "missing") {{ status.textContent = "Preview missing; reload to restart."; return; }}
  status.textContent = (s.state === "queued" ? "Queued (other transcodes running)… " : "Transcoding: ")
    + fmt(s.seconds) + " of " + fmt(s.duration);
  setTimeout(poll, started ? 2000 : 300);
}}
(useNative ? Promise.resolve() : loadHlsJs()).then(() => {{
  if (!useNative && !Hls.isSupported()) {{
    if (!canNative) {{
      status.textContent = "This browser supports neither HLS nor MSE; open the original instead.";
      return;
    }}
    useNative = true;
  }}
  poll();
}}, (e) => {{ status.textContent = e.message; }});
</script>"#,
        size = human_size(meta.len()),
        name = escape(&name),
        hls_js_url = assets::HLS_JS,
    );
    Ok(Html(page(&name, &body)).into_response())
}

#[derive(Serialize)]
struct Status {
    state: &'static str,
    segments: usize,
    seconds: f64,
    duration: Option<f64>,
    error: Option<String>,
}

/// Count segments and total duration listed in a playlist.
fn playlist_progress(m3u8: &str) -> (usize, f64) {
    m3u8.lines()
        .filter_map(|l| l.strip_prefix("#EXTINF:"))
        .filter_map(|l| l.split(',').next()?.trim().parse::<f64>().ok())
        .fold((0, 0.0), |(n, t), d| (n + 1, t + d))
}

pub async fn status(
    State(st): State<Arc<AppState>>,
    extract::Path(key): extract::Path<String>,
) -> Result<Response, AppError> {
    if !valid_key(&key) {
        return Err(AppError::not_found());
    }
    let dir = job_dir(&st, &key);
    let m3u8 = tokio::fs::read_to_string(dir.join("index.m3u8"))
        .await
        .unwrap_or_default();
    let (segments, seconds) = playlist_progress(&m3u8);
    let done = tokio::fs::metadata(dir.join("done")).await.is_ok();
    let job = st.jobs.lock().unwrap().get(&key).cloned();
    let (state, duration, error) = match (&job, done) {
        (_, true) => ("done", None, None),
        (None, false) => ("missing", None, None),
        (Some(j), false) => {
            let d = *j.duration.lock().unwrap();
            match &*j.state.lock().unwrap() {
                JobState::Queued => ("queued", d, None),
                JobState::Running => ("running", d, None),
                // The marker is written before the state flips, so a finished
                // job without one has been evicted from the cache.
                JobState::Done => ("missing", None, None),
                JobState::Failed(e) => ("failed", d, Some(e.clone())),
            }
        }
    };
    let duration = duration.or(done.then_some(seconds));
    let mut resp = axum::Json(Status {
        state,
        segments,
        seconds,
        duration,
        error,
    })
    .into_response();
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(resp)
}

/// Adjust ffmpeg's playlist for playback while it is still growing.
fn rewrite_playlist(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 64);
    for line in text.lines() {
        if let Some(t) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
            // ffmpeg raises the target whenever a longer segment appears, but
            // the spec forbids it changing in a live/EVENT playlist. Keyframes
            // are forced every SEGMENT_SECS, so only very low frame rate
            // sources overshoot; pin a generous value up front.
            let t: u32 = t.trim().parse().unwrap_or(0);
            out.push_str(&format!(
                "#EXT-X-TARGETDURATION:{}\n",
                t.max(2 * SEGMENT_SECS)
            ));
            continue;
        }
        out.push_str(line);
        out.push('\n');
        if line.trim() == "#EXTM3U" {
            // Without EXT-X-START, players join an EVENT playlist near its
            // live edge; we want playback from the beginning.
            out.push_str("#EXT-X-START:TIME-OFFSET=0,PRECISE=YES\n");
        }
    }
    out
}

pub async fn serve_hls_file(
    State(st): State<Arc<AppState>>,
    extract::Path((key, file)): extract::Path<(String, String)>,
    req: Request,
) -> Result<Response, AppError> {
    let file_ok = !file.starts_with('.')
        && file
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        && (file.ends_with(".m4s") || file.ends_with(".mp4") || file.ends_with(".m3u8"));
    if !valid_key(&key) || !file_ok {
        return Err(AppError::not_found());
    }
    let path = job_dir(&st, &key).join(&file);

    if file.ends_with(".m3u8") {
        let text = rewrite_playlist(&tokio::fs::read_to_string(&path).await?);
        return Ok((
            [
                (header::CONTENT_TYPE, "application/vnd.apple.mpegurl"),
                (header::CACHE_CONTROL, "no-cache"),
            ],
            text,
        )
            .into_response());
    }

    let mut resp = ServeFile::new(&path).oneshot(req).await.into_response();
    if resp.status().is_success() {
        let h = resp.headers_mut();
        h.insert(header::CONTENT_TYPE, HeaderValue::from_static("video/mp4"));
        h.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("max-age=3600"),
        );
    }
    Ok(resp)
}
