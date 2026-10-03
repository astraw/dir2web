//! Files embedded from `static/` and served under `/_dir2web/static/`.
//!
//! Each URL carries the upstream version, so responses are cached forever.
//! Licenses are listed in `static/THIRD-PARTY-LICENSES.yaml`.

use axum::{
    extract::Path,
    http::header,
    response::{IntoResponse, Response},
};

use crate::AppError;

/// hls.js (light build), for browsers without native HLS.
pub const HLS_JS: &str = "/_dir2web/static/hls-1.7.3.light.min.js";
/// The `<model-viewer>` web component (three.js inside), as an ES module.
pub const MODEL_VIEWER_JS: &str = "/_dir2web/static/model-viewer-4.3.1/model-viewer.min.js";
/// Directories model-viewer loads the Draco decoder and the Basis (KTX2)
/// transcoder from; by default it would fetch them from Google's CDN.
pub const DRACO_DIR: &str = "/_dir2web/static/draco-gltf-three-r183/";
pub const BASIS_DIR: &str = "/_dir2web/static/basis-three-r183/";
/// model-viewer has a meshopt decoder built in, but only enables it after
/// loading a script from its `meshoptDecoderLocation`; this one exists to
/// satisfy that, so nothing is fetched from a CDN.
pub const MESHOPT_ENABLE_JS: &str = "/_dir2web/static/meshopt-enable.js";

struct Asset {
    url: &'static str,
    mime: &'static str,
    bytes: &'static [u8],
}

const JS: &str = "text/javascript; charset=utf-8";
const WASM: &str = "application/wasm";

static ASSETS: &[Asset] = &[
    Asset {
        url: HLS_JS,
        mime: JS,
        bytes: include_bytes!("../static/hls.light.min.js"),
    },
    Asset {
        url: MODEL_VIEWER_JS,
        mime: JS,
        bytes: include_bytes!("../static/model-viewer/model-viewer.min.js"),
    },
    Asset {
        url: MESHOPT_ENABLE_JS,
        mime: JS,
        bytes: b"// Loaded by model-viewer to enable its built-in meshopt decoder.\n",
    },
    Asset {
        url: "/_dir2web/static/draco-gltf-three-r183/draco_wasm_wrapper.js",
        mime: JS,
        bytes: include_bytes!("../static/draco/draco_wasm_wrapper.js"),
    },
    Asset {
        url: "/_dir2web/static/draco-gltf-three-r183/draco_decoder.wasm",
        mime: WASM,
        bytes: include_bytes!("../static/draco/draco_decoder.wasm"),
    },
    Asset {
        url: "/_dir2web/static/basis-three-r183/basis_transcoder.js",
        mime: JS,
        bytes: include_bytes!("../static/basis/basis_transcoder.js"),
    },
    Asset {
        url: "/_dir2web/static/basis-three-r183/basis_transcoder.wasm",
        mime: WASM,
        bytes: include_bytes!("../static/basis/basis_transcoder.wasm"),
    },
];

pub async fn serve(Path(path): Path<String>) -> Result<Response, AppError> {
    let url = format!("/_dir2web/static/{path}");
    let asset = ASSETS
        .iter()
        .find(|a| a.url == url)
        .ok_or_else(AppError::not_found)?;
    Ok((
        [
            (header::CONTENT_TYPE, asset.mime),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        asset.bytes,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn referenced_urls_are_embedded() {
        let has = |url: &str| ASSETS.iter().any(|a| a.url == url);
        assert!(has(HLS_JS));
        assert!(has(MODEL_VIEWER_JS));
        assert!(has(MESHOPT_ENABLE_JS));
        // Files model-viewer requests from the decoder directories.
        for f in ["draco_wasm_wrapper.js", "draco_decoder.wasm"] {
            assert!(has(&format!("{DRACO_DIR}{f}")), "{f}");
        }
        for f in ["basis_transcoder.js", "basis_transcoder.wasm"] {
            assert!(has(&format!("{BASIS_DIR}{f}")), "{f}");
        }
    }
}
