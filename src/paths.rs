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
pub async fn resolve(root: &Path, uri_path: &str) -> Result<PathBuf, AppError> {
    let bytes: Vec<u8> = percent_decode_str(uri_path).collect();
    let mut path = root.to_path_buf();
    for comp in bytes.split(|b| *b == b'/') {
        match comp {
            b"" | b"." => continue,
            b".." => return Err(AppError::not_found()),
            _ if comp.contains(&0) => return Err(AppError::not_found()),
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

#[cfg(test)]
mod tests {
    use super::resolve;

    #[tokio::test]
    async fn dotfiles_resolve_and_escapes_do_not() {
        let tmp = std::env::temp_dir().join(format!("dir2web-paths-{}", std::process::id()));
        let root = tmp.join("root");
        std::fs::create_dir_all(root.join(".hidden/.inner")).unwrap();
        std::fs::write(root.join(".hidden/.inner/.file"), "x").unwrap();
        std::fs::write(root.join("..."), "x").unwrap();
        std::fs::write(root.join("..foo"), "x").unwrap();
        std::fs::write(tmp.join("outside"), "x").unwrap();
        std::os::unix::fs::symlink(tmp.join("outside"), root.join(".link")).unwrap();
        let root = std::fs::canonicalize(&root).unwrap();

        for ok in [
            "/.hidden/",
            "/.hidden/.inner/.file",
            "/%2Ehidden/.inner",
            "/...",
            "/..foo",
        ] {
            assert!(resolve(&root, ok).await.is_ok(), "{ok} should resolve");
        }
        for bad in [
            "/../outside",
            "/.hidden/../../outside",
            "/%2E%2E/outside",
            "/.link",
        ] {
            assert!(
                resolve(&root, bad).await.is_err(),
                "{bad} should not resolve"
            );
        }
        std::fs::remove_dir_all(&tmp).unwrap();
    }
}
