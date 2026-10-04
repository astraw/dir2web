# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0](https://github.com/astraw/dir2web/releases/tag/v0.1.0) - 2026-10-04

### Added

- view 3D models (.glb, .gltf) in the browser (Claude Opus 5.5)

### Fixed

- fill in the valuable license text missing from THIRD-PARTY-LICENSES (Claude Opus 5.5)

### Other

- Add dir2web: directory index with video thumbnails and HLS previews (Claude Opus 5.5)
- Add Cargo.lock (Claude Opus 5.5)
- Reserve a fixed 160x90 box for thumbnails (Claude Opus 5.5)
- Vendor hls.js 1.7.3 light build (Claude Opus 5.5)
- Play previews in Chrome and Firefox via hls.js (Claude Opus 5.5)
- change default port
- Serve a directory's index.html instead of the generated listing (Claude Opus 5.5)
- Add a small dir2web footer to generated listings (Claude Opus 5.5)
- Add crates.io package metadata and MIT/Apache-2.0 license files (Claude Opus 5.5)
- Limit the cache size with LRU eviction, default 10G (Claude Opus 5.5)
- Render Markdown files, and a directory's README below its listing (Claude Opus 5.5)
- Update Cargo.lock for pulldown-cmark (Claude Opus 5.5)
- Show the dir2web footer on every generated page (Claude Opus 5.5)
- bundle third-party license texts for release archives (Claude Opus 5.5)
- run fmt, clippy and tests on Linux and macOS (Claude Opus 5.5)
- release with release-plz and cargo-dist, changelog via git-cliff (Claude Opus 5.5)
- document binary installs, contributing and releasing (Claude Opus 5.5)
- drop .fmf/.ufmf from the README's plans (Claude Opus 5.5)
- keep vendored-JS license entries in static/THIRD-PARTY-LICENSES.yaml (Claude Opus 5.5)
- vendor model-viewer 4.3.1 with Draco and Basis decoders (Claude Opus 5.5)
