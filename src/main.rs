//! dir2web: browse a directory tree over HTTP with Apache-style indices,
//! video thumbnails, and quick-start transcoded HLS previews.

mod assets;
mod cache;
mod hls;
mod html;
mod listing;
mod markdown;
mod media;
mod model;
mod paths;

use std::{
    collections::HashMap,
    net::SocketAddr,
    path::PathBuf,
    sync::{Arc, Mutex},
};

use anyhow::Context;
use axum::{
    Router,
    extract::{Request, State},
    http::{StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::get,
};
use clap::Parser;
use tokio::sync::{Notify, Semaphore};
use tower::ServiceExt;
use tower_http::services::ServeFile;

#[derive(Parser, Debug)]
#[command(version, about)]
struct Cli {
    /// Base directory to serve.
    #[arg(default_value = ".")]
    root: PathBuf,
    /// Address to listen on. There is no authentication, so only bind to
    /// trusted networks.
    #[arg(long, default_value = "127.0.0.1:37326")]
    listen: SocketAddr,
    /// Cache directory (default: the user cache dir, e.g. ~/.cache/dir2web).
    #[arg(long)]
    cache_dir: Option<PathBuf>,
    /// Show and serve dotfiles.
    #[arg(long)]
    hidden: bool,
    /// Maximum width of preview videos, in pixels.
    #[arg(long, default_value_t = 640)]
    preview_width: u32,
    /// Maximum frame rate of preview videos.
    #[arg(long, default_value_t = 30)]
    preview_fps_max: u32,
    /// x264 CRF of preview videos (higher is smaller and worse).
    #[arg(long, default_value_t = 28)]
    preview_crf: u32,
    /// Width of thumbnails, in pixels.
    #[arg(long, default_value_t = 320)]
    thumb_width: u32,
    /// Maximum number of simultaneous preview transcodes.
    #[arg(long, default_value_t = 2)]
    max_transcodes: usize,
    /// Maximum number of simultaneous thumbnail extractions.
    #[arg(long, default_value_t = 4)]
    max_thumbnailers: usize,
    /// Maximum cache size, e.g. 10G or 500M (binary units); least recently
    /// used thumbnails and previews are evicted beyond it. 0 means no limit.
    #[arg(long, default_value = "10G", value_parser = cache::parse_size)]
    max_cache_size: u64,
    /// Keep running when stdin reaches end-of-file. By default, when stdin is
    /// a pipe or socket (as when run by ssh without -t), its closing shuts
    /// the server down, so a dropped ssh session does not leave it running.
    #[arg(long)]
    no_exit_on_stdin_eof: bool,
}

#[derive(Debug, Clone)]
pub struct PreviewConfig {
    pub width: u32,
    pub fps_max: u32,
    pub crf: u32,
    pub thumb_width: u32,
}

pub struct AppState {
    /// Canonicalized base directory.
    pub root: PathBuf,
    pub cache: PathBuf,
    pub show_hidden: bool,
    pub cfg: PreviewConfig,
    pub jobs: Mutex<HashMap<String, Arc<hls::Job>>>,
    pub transcode_sem: Arc<Semaphore>,
    pub thumb_sem: Semaphore,
    pub thumb_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Cache size limit in bytes (0: unlimited).
    pub max_cache_bytes: u64,
    /// Notified when the cache has grown, to trigger an eviction sweep.
    pub cache_sweep: Arc<Notify>,
}

/// An error rendered as a plain-text HTTP response.
pub struct AppError(pub StatusCode, pub String);

impl AppError {
    pub fn not_found() -> Self {
        AppError(StatusCode::NOT_FOUND, "Not Found".into())
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        (
            self.0,
            [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
            self.1,
        )
            .into_response()
    }
}

impl From<anyhow::Error> for AppError {
    fn from(e: anyhow::Error) -> Self {
        tracing::error!("{e:#}");
        AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("{e:#}"))
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        match e.kind() {
            std::io::ErrorKind::NotFound => AppError::not_found(),
            std::io::ErrorKind::PermissionDenied => {
                AppError(StatusCode::FORBIDDEN, "Forbidden".into())
            }
            _ => anyhow::Error::from(e).into(),
        }
    }
}

/// Is `name` present as a key (with or without a value) in the query string?
pub fn query_has(query: Option<&str>, name: &str) -> bool {
    query_get(query, name).is_some()
}

/// Value of `name` in the query string ("" for a bare key).
pub fn query_get<'a>(query: Option<&'a str>, name: &str) -> Option<&'a str> {
    query?.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
        (k == name).then_some(v)
    })
}

async fn serve_path(State(st): State<Arc<AppState>>, req: Request) -> Result<Response, AppError> {
    let uri_path = req.uri().path().to_owned();
    let query = req.uri().query().map(str::to_owned);
    let query = query.as_deref();
    let path = paths::resolve(&st.root, st.show_hidden, &uri_path).await?;
    let meta = tokio::fs::metadata(&path).await?;

    if meta.is_dir() {
        if !uri_path.ends_with('/') {
            let mut to = format!("{uri_path}/");
            if let Some(q) = query {
                to.push('?');
                to.push_str(q);
            }
            return Ok(Redirect::permanent(&to).into_response());
        }
        // Like Apache's DirectoryIndex: a directory's own index.html wins
        // over the generated listing (unless it resolves outside the root).
        if let Ok(index) = tokio::fs::canonicalize(path.join("index.html")).await
            && index.starts_with(&st.root)
            && tokio::fs::metadata(&index).await.is_ok_and(|m| m.is_file())
        {
            return Ok(ServeFile::new(&index).oneshot(req).await.into_response());
        }
        return listing::render(&st, &path, &uri_path, query).await;
    }
    if query_has(query, "thumb") {
        return media::thumbnail(&st, &path, &meta).await;
    }
    if query_has(query, "play") {
        return hls::play_page(&st, &path, &meta, &uri_path).await;
    }
    if query_has(query, "view") {
        if markdown::is_markdown(path.as_os_str()) {
            return markdown::view_page(&path, &meta, &uri_path).await;
        }
        if model::is_model(path.as_os_str()) {
            return Ok(model::view_page(&path, &meta, &uri_path));
        }
    }
    Ok(ServeFile::new(&path).oneshot(req).await.into_response())
}

/// Resolves on SIGINT, SIGTERM or SIGHUP, or when `stdin_closed` fires.
/// Returning from main drops all tasks, which kills their ffmpeg children.
async fn shutdown_signal(stdin_closed: Option<tokio::sync::oneshot::Receiver<()>>) {
    use tokio::signal::unix::{SignalKind, signal};
    let mut term = signal(SignalKind::terminate()).expect("installing SIGTERM handler");
    let mut hup = signal(SignalKind::hangup()).expect("installing SIGHUP handler");
    let stdin_closed = async {
        match stdin_closed {
            Some(rx) => {
                let _ = rx.await;
            }
            None => std::future::pending().await,
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
        _ = hup.recv() => {}
        _ = stdin_closed => {
            tracing::info!(
                "stdin closed; shutting down (pass --no-exit-on-stdin-eof to keep running)"
            );
        }
    }
    tracing::info!("shutting down");
}

/// If stdin is a pipe or socket, which is what ssh gives a command run
/// without a terminal, spawns a thread that reads it to end-of-file and then
/// fires the returned receiver. When the ssh session goes away, sshd closes
/// the pipe. A terminal (covered by SIGHUP), /dev/null or a regular file is
/// not watched. A plain thread is used because a pending
/// `tokio::io::stdin()` read can hold up runtime shutdown.
fn watch_stdin_eof() -> Option<tokio::sync::oneshot::Receiver<()>> {
    // SAFETY: fstat only writes into the zeroed stat buffer we pass.
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    if unsafe { libc::fstat(libc::STDIN_FILENO, &mut st) } != 0 {
        return None;
    }
    let fmt = st.st_mode & libc::S_IFMT;
    if fmt != libc::S_IFIFO && fmt != libc::S_IFSOCK {
        return None;
    }
    let (tx, rx) = tokio::sync::oneshot::channel();
    std::thread::Builder::new()
        .name("stdin-eof".into())
        .spawn(move || {
            use std::io::Read;
            let mut buf = [0u8; 4096];
            let mut stdin = std::io::stdin().lock();
            loop {
                match stdin.read(&mut buf) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                    // An unreadable stdin says nothing about the session.
                    Err(_) => return,
                }
            }
            let _ = tx.send(());
        })
        .ok()?;
    Some(rx)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "dir2web=info,tower_http=warn".into()),
        )
        .init();
    let cli = Cli::parse();

    let root = std::fs::canonicalize(&cli.root)
        .with_context(|| format!("opening root directory {}", cli.root.display()))?;
    anyhow::ensure!(root.is_dir(), "{} is not a directory", root.display());
    let cache = match cli.cache_dir {
        Some(c) => c,
        None => dirs::cache_dir()
            .context("cannot determine user cache directory; pass --cache-dir")?
            .join("dir2web"),
    };
    std::fs::create_dir_all(cache.join("thumbs"))
        .with_context(|| format!("creating cache directory {}", cache.display()))?;
    std::fs::create_dir_all(cache.join("hls"))?;

    for tool in ["ffmpeg", "ffprobe"] {
        if std::process::Command::new(tool)
            .arg("-version")
            .output()
            .is_err()
        {
            tracing::warn!("{tool} not found in PATH; thumbnails and previews will fail");
        }
    }

    let st = Arc::new(AppState {
        root,
        cache,
        show_hidden: cli.hidden,
        cfg: PreviewConfig {
            width: cli.preview_width,
            fps_max: cli.preview_fps_max,
            crf: cli.preview_crf,
            thumb_width: cli.thumb_width,
        },
        jobs: Mutex::new(HashMap::new()),
        transcode_sem: Arc::new(Semaphore::new(cli.max_transcodes.max(1))),
        thumb_sem: Semaphore::new(cli.max_thumbnailers.max(1)),
        thumb_locks: Mutex::new(HashMap::new()),
        max_cache_bytes: cli.max_cache_size,
        cache_sweep: Arc::new(Notify::new()),
    });
    tokio::spawn(cache::sweeper(st.clone()));

    let app = Router::new()
        .route("/_dir2web/status/{key}", get(hls::status))
        .route("/_dir2web/static/{*path}", get(assets::serve))
        .route("/_dir2web/hls/{key}/{file}", get(hls::serve_hls_file))
        .fallback(serve_path)
        .with_state(st.clone());

    let listener = tokio::net::TcpListener::bind(cli.listen)
        .await
        .with_context(|| format!("binding {}", cli.listen))?;
    let cache_dir = st.cache.clone();
    let used = tokio::task::spawn_blocking(move || cache::usage(&cache_dir)).await?;
    let limit = match st.max_cache_bytes {
        0 => "unlimited".to_owned(),
        n => html::human_size(n),
    };
    tracing::info!(
        "serving {} on http://{} (cache: {}, {} of {limit})",
        st.root.display(),
        cli.listen,
        st.cache.display(),
        html::human_size(used)
    );
    let stdin_closed = if cli.no_exit_on_stdin_eof {
        None
    } else {
        watch_stdin_eof()
    };
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(stdin_closed))
        .await?;
    Ok(())
}
