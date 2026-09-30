//! Cache size limit: least-recently-used eviction of thumbnails and previews.
//!
//! Each thumbnail file and each preview directory is one entry. Its mtime is
//! its "last used" time: set when written, and refreshed (`touch`) whenever
//! it is served, since atime is unreliable under relatime/noatime.

use std::{
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

use crate::{AppState, hls, html::human_size};

/// After exceeding the limit, evict down to this fraction of it, so that
/// sweeps are not triggered by every new file.
const LOW_WATER: f64 = 0.9;

/// Entries used this recently are never evicted, so a preview being watched
/// survives even if it alone exceeds the limit; the cache may temporarily
/// exceed the limit by what is in active use.
const GRACE: Duration = Duration::from_secs(5 * 60);

/// Minimum time between sweeps; requests arriving meanwhile coalesce.
const SWEEP_INTERVAL: Duration = Duration::from_secs(5);

/// Parse a size such as `10G`, `500MB` or `1.5TiB`. Units are binary
/// (K = 1024) with or without a trailing `B`/`iB`.
pub fn parse_size(s: &str) -> Result<u64, String> {
    let s = s.trim();
    let split = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.'))
        .unwrap_or(s.len());
    let (num, unit) = s.split_at(split);
    let num: f64 = num.parse().map_err(|_| format!("invalid size {s:?}"))?;
    let mult: u64 = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "b" => 1,
        "k" | "kb" | "kib" => 1 << 10,
        "m" | "mb" | "mib" => 1 << 20,
        "g" | "gb" | "gib" => 1 << 30,
        "t" | "tb" | "tib" => 1 << 40,
        _ => return Err(format!("unknown unit in size {s:?}")),
    };
    Ok((num * mult as f64) as u64)
}

/// Mark a cache entry (file or directory) as just used.
pub fn touch(path: &Path) {
    let _ = std::fs::File::open(path).and_then(|f| f.set_modified(SystemTime::now()));
}

struct Entry {
    path: PathBuf,
    is_dir: bool,
    bytes: u64,
    last_used: SystemTime,
}

/// Space actually used on disk, like `du`.
fn disk_bytes(meta: &std::fs::Metadata) -> u64 {
    meta.blocks() * 512
}

fn scan(cache: &Path) -> Vec<Entry> {
    let mut out = Vec::new();
    for (sub, is_dir) in [("thumbs", false), ("hls", true)] {
        let Ok(rd) = std::fs::read_dir(cache.join(sub)) else {
            continue;
        };
        for de in rd.flatten() {
            let Ok(meta) = de.metadata() else { continue };
            let mut bytes = disk_bytes(&meta);
            if is_dir {
                if !meta.is_dir() {
                    continue;
                }
                if let Ok(files) = std::fs::read_dir(de.path()) {
                    bytes += files
                        .flatten()
                        .filter_map(|f| f.metadata().ok())
                        .map(|m| disk_bytes(&m))
                        .sum::<u64>();
                }
            }
            out.push(Entry {
                path: de.path(),
                is_dir,
                bytes,
                last_used: meta.modified().unwrap_or(SystemTime::UNIX_EPOCH),
            });
        }
    }
    out
}

/// Total bytes currently used by the cache.
pub fn usage(cache: &Path) -> u64 {
    scan(cache).iter().map(|e| e.bytes).sum()
}

/// Evict least-recently-used entries until the cache fits the limit.
fn sweep(st: &AppState) {
    let max = st.max_cache_bytes;
    if max == 0 {
        return;
    }
    let mut entries = scan(&st.cache);
    let mut total: u64 = entries.iter().map(|e| e.bytes).sum();
    if total <= max {
        return;
    }
    // Taken after the scan: a job's directory is created and registered
    // under one lock, so any directory the scan saw is registered by now.
    let active = hls::active_keys(st);
    entries.sort_by_key(|e| e.last_used);
    let target = (max as f64 * LOW_WATER) as u64;
    let recent = SystemTime::now() - GRACE;
    let (mut n, mut freed) = (0, 0);
    for e in entries {
        if total <= target {
            break;
        }
        let name = e.path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if e.last_used > recent || (e.is_dir && active.contains(name)) {
            continue;
        }
        let res = if e.is_dir {
            std::fs::remove_dir_all(&e.path)
        } else {
            std::fs::remove_file(&e.path)
        };
        match res {
            Ok(()) => {
                total -= e.bytes;
                freed += e.bytes;
                n += 1;
            }
            Err(err) => tracing::warn!("cache: removing {}: {err}", e.path.display()),
        }
    }
    if n == 0 {
        tracing::debug!(
            "cache: {} of {}, but nothing evictable",
            human_size(total),
            human_size(max)
        );
        return;
    }
    tracing::info!(
        "cache: evicted {n} entries ({}); now {} of {}",
        human_size(freed),
        human_size(total),
        human_size(max)
    );
}

/// Background task running a sweep at startup and whenever
/// `st.cache_sweep` is notified, at most once per `SWEEP_INTERVAL`.
pub async fn sweeper(st: Arc<AppState>) {
    loop {
        let st2 = st.clone();
        let _ = tokio::task::spawn_blocking(move || sweep(&st2)).await;
        tokio::time::sleep(SWEEP_INTERVAL).await;
        st.cache_sweep.notified().await;
    }
}

#[cfg(test)]
mod tests {
    use super::parse_size;

    #[test]
    fn sizes() {
        assert_eq!(parse_size("0"), Ok(0));
        assert_eq!(parse_size("1234"), Ok(1234));
        assert_eq!(parse_size("10G"), Ok(10 << 30));
        assert_eq!(parse_size("10GB"), Ok(10 << 30));
        assert_eq!(parse_size("10 GiB"), Ok(10 << 30));
        assert_eq!(parse_size("500m"), Ok(500 << 20));
        assert_eq!(parse_size("1.5T"), Ok(3 << 39));
        assert!(parse_size("ten").is_err());
        assert!(parse_size("10X").is_err());
    }
}
