//! Viewer page for 3D models (glTF / GLB), using the embedded
//! `<model-viewer>` web component.

use std::{ffi::OsStr, fs::Metadata, path::Path};

use axum::response::{Html, IntoResponse, Response};

use crate::{
    assets,
    html::{escape, human_size, page},
};

const EXTS: &[&str] = &["glb", "gltf"];

pub fn is_model(name: &OsStr) -> bool {
    Path::new(name)
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| EXTS.iter().any(|m| m.eq_ignore_ascii_case(ext)))
}

pub fn view_page(path: &Path, meta: &Metadata, uri_path: &str) -> Response {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    // Relative, so a .gltf's external .bin and texture files resolve next to it.
    let raw_href = uri_path.rsplit('/').next().unwrap_or("");
    let body = format!(
        r#"<p><a href="./">⬑ Back to directory</a> · <a href="{raw_href}">original</a> ({size})</p>
<h1>{name}</h1>
<script>
self.ModelViewerElement = self.ModelViewerElement || {{}};
self.ModelViewerElement.dracoDecoderLocation = "{draco}";
self.ModelViewerElement.ktx2TranscoderLocation = "{basis}";
self.ModelViewerElement.meshoptDecoderLocation = "{meshopt}";
</script>
<script type="module" src="{model_viewer}"></script>
<model-viewer id="m" src="{raw_href}" alt="{name}" camera-controls touch-action="pan-y"
  autoplay shadow-intensity="1" environment-image="neutral" interaction-prompt="none"></model-viewer>
<p id="status">Loading…</p>
<script>
const m = document.getElementById("m");
const status = document.getElementById("status");
// model-viewer reports "load" even when it cannot render.
const webgl = document.createElement("canvas").getContext("webgl2");
m.addEventListener("progress", (e) => {{
  if (e.detail.totalProgress < 1) {{
    status.textContent = "Loading… " + Math.round(100 * e.detail.totalProgress) + "%";
  }}
}});
m.addEventListener("load", () => {{
  if (!webgl) {{
    status.textContent = "This browser cannot display 3D models (WebGL 2 is unavailable).";
    return;
  }}
  const d = m.getDimensions();
  status.textContent = "Loaded: " + [d.x, d.y, d.z].map((v) => v.toPrecision(3)).join(" × ")
    + " m. Drag to rotate, scroll to zoom, right-drag or two fingers to pan.";
}});
m.addEventListener("error", (e) => {{
  const err = e.detail && e.detail.sourceError;
  status.textContent = "Could not load the model: " + (err ? err.message || err : "unknown error");
}});
</script>"#,
        size = human_size(meta.len()),
        name = escape(&name),
        draco = assets::DRACO_DIR,
        basis = assets::BASIS_DIR,
        meshopt = assets::MESHOPT_ENABLE_JS,
        model_viewer = assets::MODEL_VIEWER_JS,
    );
    Html(page(&name, &body)).into_response()
}
