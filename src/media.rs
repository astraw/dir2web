//! Video detection, cache keys, ffprobe, and thumbnail extraction.

use std::{
    ffi::{OsStr, OsString},
    fs::Metadata,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::UNIX_EPOCH,
};

use anyhow::Context;
use axum::{
    http::header,
    response::{IntoResponse, Response},
};
use sha2::{Digest, Sha256};
use tokio::process::Command;

use crate::{AppError, AppState};

const VIDEO_EXTS: &[&str] = &[
    "mp4", "m4v", "mov", "mkv", "webm", "avi", "mts", "m2ts", "ts", "mpg", "mpeg", "wmv", "flv",
    "3gp", "ogv", "h264", "264", "h265", "265", "hevc",
];

pub fn is_video(name: &OsStr) -> bool {
    Path::new(name)
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| VIDEO_EXTS.iter().any(|v| v.eq_ignore_ascii_case(ext)))
}

/// A stable cache key for (kind, output parameters, source file identity).
/// Changing the source file (size or mtime) or the parameters yields a new key.
pub fn cache_key(kind: &str, params: &str, path: &Path, meta: &Metadata) -> String {
    let mtime_ns = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut h = Sha256::new();
    for part in [
        kind.as_bytes(),
        params.as_bytes(),
        path.as_os_str().as_bytes(),
        meta.len().to_string().as_bytes(),
        mtime_ns.to_string().as_bytes(),
    ] {
        h.update(part);
        h.update([0u8]);
    }
    hex::encode(&h.finalize()[..16])
}

/// ffmpeg input argument for a local file. The `file:` prefix stops ffmpeg
/// from interpreting names containing `:` as protocol URLs.
pub fn ffmpeg_input(path: &Path) -> OsString {
    let mut s = OsString::from("file:");
    s.push(path.as_os_str());
    s
}

/// A subprocess that cannot outlive us: it is killed when its handle is
/// dropped, and (on Linux) when this process dies, even by SIGKILL.
pub fn child_command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    cmd.stdin(Stdio::null()).kill_on_drop(true);
    #[cfg(target_os = "linux")]
    // SAFETY: prctl is async-signal-safe and touches no parent state.
    unsafe {
        cmd.pre_exec(|| {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    cmd
}

/// Container duration in seconds, if ffprobe can tell.
pub async fn probe_duration(path: &Path) -> Option<f64> {
    let out = child_command("ffprobe")
        .args([
            "-v",
            "error",
            "-show_entries",
            "format=duration",
            "-of",
            "default=nw=1:nk=1",
        ])
        .arg(ffmpeg_input(path))
        .output()
        .await
        .ok()?;
    let d: f64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
    (d.is_finite() && d > 0.0).then_some(d)
}

const PLACEHOLDER_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" width="160" height="90" viewBox="0 0 160 90"><rect width="160" height="90" fill="#ddd"/><polygon points="68,30 68,60 94,45" fill="#999"/></svg>"##;

fn placeholder() -> Response {
    (
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (header::CACHE_CONTROL, "max-age=300"),
        ],
        PLACEHOLDER_SVG,
    )
        .into_response()
}

async fn extract_frame(src: &Path, at: f64, width: u32, out: &Path) -> anyhow::Result<()> {
    let status = child_command("ffmpeg")
        .args([
            "-nostdin",
            "-hide_banner",
            "-v",
            "error",
            "-y",
            "-ss",
            &format!("{at:.3}"),
        ])
        .arg("-i")
        .arg(ffmpeg_input(src))
        .args(["-map", "0:v:0", "-frames:v", "1", "-vf"])
        .arg(format!("scale=w='min({width},iw)':h=-2"))
        .args(["-q:v", "5", "-f", "image2", "-update", "1"])
        .arg(out)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .context("running ffmpeg")?;
    let ok = status.success() && tokio::fs::metadata(out).await.is_ok_and(|m| m.len() > 0);
    anyhow::ensure!(ok, "ffmpeg produced no frame at {at:.3}s ({status})");
    Ok(())
}

async fn generate_thumb(st: &AppState, src: &Path, jpg: &Path) -> anyhow::Result<()> {
    let _permit = st.thumb_sem.acquire().await?;
    let tmp = jpg.with_extension("tmp.jpg");
    let at = probe_duration(src).await.map(|d| d * 0.1).unwrap_or(0.0);
    let res = match extract_frame(src, at, st.cfg.thumb_width, &tmp).await {
        Err(_) if at > 0.0 => extract_frame(src, 0.0, st.cfg.thumb_width, &tmp).await,
        r => r,
    };
    match res {
        Ok(()) => Ok(tokio::fs::rename(&tmp, jpg).await?),
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            Err(e)
        }
    }
}

pub async fn thumbnail(st: &AppState, src: &Path, meta: &Metadata) -> Result<Response, AppError> {
    let key = cache_key("thumb-v1", &st.cfg.thumb_width.to_string(), src, meta);
    let dir = st.cache.join("thumbs");
    let jpg: PathBuf = dir.join(format!("{key}.jpg"));
    let fail = dir.join(format!("{key}.fail"));

    // Serialize work per key so concurrent requests extract only once.
    let lock = {
        let mut locks = st.thumb_locks.lock().unwrap();
        locks.entry(key.clone()).or_default().clone()
    };
    let guard = lock.lock().await;
    if tokio::fs::metadata(&jpg).await.is_err()
        && tokio::fs::metadata(&fail).await.is_err()
        && let Err(e) = generate_thumb(st, src, &jpg).await
    {
        tracing::warn!("thumbnail for {} failed: {e:#}", src.display());
        let _ = tokio::fs::write(&fail, format!("{e:#}\n")).await;
    }
    drop(guard);
    {
        let mut locks = st.thumb_locks.lock().unwrap();
        if Arc::strong_count(&lock) <= 2 {
            locks.remove(&key);
        }
    }

    match tokio::fs::read(&jpg).await {
        Ok(bytes) => Ok((
            [
                (header::CONTENT_TYPE, "image/jpeg"),
                (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            ],
            bytes,
        )
            .into_response()),
        Err(_) => Ok(placeholder()),
    }
}
