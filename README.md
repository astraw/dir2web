# dir2web

Browse a directory tree over HTTP with Apache-style index pages. Videos get a
thumbnail in the listing, and clicking one opens a player that starts a
low-resolution preview within a second or so, even for multi-GB files.

```sh
cargo install dir2web   # or, from a checkout: cargo build --release
dir2web /path/to/base --listen 0.0.0.0:8080
```

There is **no authentication**: anyone who can reach the port can read every
file under the base directory. The default `--listen` is `127.0.0.1:37326`;
only bind wider on a trusted LAN. See `--help` for the preview and
concurrency knobs.

Requires `ffmpeg` and `ffprobe` (with libx264) in `PATH`.

## How it works

- `/<path>/` — directory index (sortable by name, mtime, size; dotfiles hidden
  unless `--hidden`), or the directory's own `index.html` if it has one.
  A `README.md` in the directory is rendered below the listing.
  Paths resolving outside the base directory, including via symlinks, are 404.
- `/<path>` — the original file, with HTTP range support (so browsers can
  seek in directly playable files).
- `/<path>?view` — a Markdown file (`.md`, `.markdown`, …) rendered as
  GitHub-flavoured HTML, up to 16 MiB. Raw HTML is passed through unsanitized,
  since the served files are trusted. Headings get GitHub-style anchors, and
  relative links to other Markdown files open rendered too.
- `/<path>?thumb` — JPEG thumbnail from ~10% into the video, extracted lazily
  and cached.
- `/<path>?play` — player page. Starts (or joins) a transcode of the video to
  an HLS event playlist of 2 s fMP4 segments (H.264 ≤640 px wide, ≤30 fps,
  AAC). The page polls `/_dir2web/status/<key>` and starts playback once the
  first segment exists; the part transcoded so far is seekable. A finished
  preview is complete-seekable and reused across restarts.

The cache lives in `~/.cache/dir2web` (`$XDG_CACHE_HOME`, or `--cache-dir`).
Entries are keyed by a hash of the canonical path, size, mtime and output
settings, so changed files or settings produce new entries. The cache is kept
under `--max-cache-size` (default `10G`; `0` for no limit) by evicting the
least recently used thumbnails and previews, down to 90% of the limit. Entries
used in the last 5 minutes and transcodes in progress are never evicted, so the
cache can briefly exceed the limit by what is in active use.

Safari plays the HLS preview natively. Other browsers use a vendored copy of
[hls.js](https://github.com/video-dev/hls.js) (light build, Apache-2.0, in
`static/`), compiled into the binary. Chrome's own native HLS is deliberately
not used: it mishandles the growing playlist (no duration, seeks ignored).
Append `&player=hlsjs` or `&player=native` to a player URL to force either.

## Browser test

`tests/browser/play.mjs` drives the player page in headless Chromium,
Firefox or (Linux) WebKit via Playwright, logging playback start, seeks and
buffering while the transcode runs. It expects a video of at least 240 s.

```sh
cd tests/browser && npm i --no-save playwright && npx playwright install chromium firefox webkit
node play.mjs firefox 'http://127.0.0.1:37326/some.mp4?play'
```

## Not yet

- Scrubbing beyond the transcoded portion (publish the full VOD playlist up
  front and produce segments on demand with `-ss`).
- `.fmf` / `.ufmf` via strand-braid.
- Authentication.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or
[MIT license](LICENSE-MIT) at your option.

The embedded [hls.js](https://github.com/video-dev/hls.js) in `static/` is
Apache-2.0; see `static/hls.js-LICENSE`.
