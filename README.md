# dir2web

Browse a directory tree over HTTP with Apache-style index pages. Videos get a
thumbnail in the listing, and clicking one opens a player that starts a
low-resolution preview within a second or so, even for multi-GB files.

```sh
cargo build --release
./target/release/dir2web /path/to/base --listen 0.0.0.0:8080
```

There is **no authentication**: anyone who can reach the port can read every
file under the base directory. The default `--listen` is `127.0.0.1:8080`;
only bind wider on a trusted LAN. See `--help` for the preview and
concurrency knobs.

Requires `ffmpeg` and `ffprobe` (with libx264) in `PATH`.

## How it works

- `/<path>/` — directory index (sortable by name, mtime, size; dotfiles hidden
  unless `--hidden`). Paths resolving outside the base directory, including
  via symlinks, are 404.
- `/<path>` — the original file, with HTTP range support (so browsers can
  seek in directly playable files).
- `/<path>?thumb` — JPEG thumbnail from ~10% into the video, extracted lazily
  and cached.
- `/<path>?play` — player page. Starts (or joins) a transcode of the video to
  an HLS event playlist of 2 s fMP4 segments (H.264 ≤640 px wide, ≤30 fps,
  AAC). The page polls `/_dir2web/status/<key>` and starts playback once the
  first segment exists; the part transcoded so far is seekable. A finished
  preview is complete-seekable and reused across restarts.

The cache lives in `~/.cache/dir2web` (`$XDG_CACHE_HOME`, or `--cache-dir`).
Entries are keyed by a hash of the canonical path, size, mtime and output
settings, so changed files or settings produce new entries. Nothing is evicted
yet; delete the directory to reclaim space.

Playback currently relies on native HLS, i.e. Safari.

## Not yet

- Scrubbing beyond the transcoded portion (publish the full VOD playlist up
  front and produce segments on demand with `-ss`).
- `hls.js` for Chrome/Firefox.
- Cache size limit / LRU eviction.
- `.fmf` / `.ufmf` via strand-braid.
- Authentication.
