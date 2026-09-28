//! Mapping request paths onto the filesystem, and back into URLs.

use std::{
    ffi::OsStr,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, percent_decode_str, percent_encode};

use crate::AppError;

/// Characters left unescaped in a URL path segment.
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Percent-encode a file name (arbitrary bytes) for use as one path segment.
pub fn encode_segment(name: &OsStr) -> String {
    percent_encode(name.as_bytes(), SEGMENT).to_string()
}

/// Resolve a raw (percent-encoded) request path to a canonical path inside
/// `root`. Anything outside `root`, including via symlinks, is "not found".
pub async fn resolve(root: &Path, show_hidden: bool, uri_path: &str) -> Result<PathBuf, AppError> {
    let bytes: Vec<u8> = percent_decode_str(uri_path).collect();
    let mut path = root.to_path_buf();
    for comp in bytes.split(|b| *b == b'/') {
        match comp {
            b"" | b"." => continue,
            b".." => return Err(AppError::not_found()),
            _ if comp.contains(&0) => return Err(AppError::not_found()),
            _ if !show_hidden && comp[0] == b'.' => return Err(AppError::not_found()),
            _ => path.push(OsStr::from_bytes(comp)),
        }
    }
    let canon = tokio::fs::canonicalize(&path)
        .await
        .map_err(AppError::from)?;
    if !canon.starts_with(root) {
        return Err(AppError::not_found());
    }
    Ok(canon)
}
