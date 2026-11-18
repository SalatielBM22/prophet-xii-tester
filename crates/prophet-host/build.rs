// SPDX-License-Identifier: GPL-3.0-only
//! Embeds compiled frontend artifacts from `../../ui/dist` into `prophet-host` so VST3, CLAP,
//! LV2, and Desktop Standalone targets serve the offline React panel with zero local HTTP server.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn mime_for_path(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".html") || lower.ends_with(".htm") {
        "text/html; charset=utf-8"
    } else if lower.ends_with(".js") || lower.ends_with(".mjs") {
        "application/javascript; charset=utf-8"
    } else if lower.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if lower.ends_with(".wasm") {
        "application/wasm"
    } else if lower.ends_with(".json") || lower.ends_with(".map") {
        "application/json; charset=utf-8"
    } else if lower.ends_with(".svg") {
        "image/svg+xml"
    } else if lower.ends_with(".png") {
        "image/png"
    } else if lower.ends_with(".woff2") {
        "font/woff2"
    } else {
        "application/octet-stream"
    }
}

fn collect_files(root: &Path, dir: &Path, out: &mut BTreeMap<String, PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, out);
        } else if path.is_file() {
            if let Ok(rel) = path.strip_prefix(root) {
                let mut web_path = String::from("/");
                let mut first = true;
                for comp in rel.components() {
                    if !first {
                        web_path.push('/');
                    }
                    first = false;
                    web_path.push_str(&comp.as_os_str().to_string_lossy());
                }
                println!("cargo:rerun-if-changed={}", path.display());
                out.insert(web_path, path);
            }
        }
    }
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let ui_dist = manifest_dir.join("../../ui/dist");
    println!("cargo:rerun-if-changed={}", ui_dist.display());

    let mut files: BTreeMap<String, PathBuf> = BTreeMap::new();
    if ui_dist.is_dir() {
        collect_files(&ui_dist, &ui_dist, &mut files);
    }

    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let dest = out_dir.join("embedded_ui_assets.rs");

    let mut code = String::from(
        "pub static EMBEDDED_UI_ASSETS: &[EmbeddedUiAsset] = &[\n",
    );
    if !files.contains_key("/index.html") {
        let fallback = b"<!doctype html><html><head><meta charset=\"utf-8\"><title>PROPHET-XII</title></head><body><div id=\"root\">PROPHET-XII UI bundle missing</div></body></html>";
        code.push_str(&format!(
            "    EmbeddedUiAsset {{ path: \"/index.html\", mime: \"text/html; charset=utf-8\", bytes: &{:?} }},\n",
            fallback
        ));
    }
    for (web_path, disk_path) in &files {
        let abs = fs::canonicalize(disk_path).unwrap_or_else(|_| disk_path.clone());
        let mime = mime_for_path(web_path);
        code.push_str(&format!(
            "    EmbeddedUiAsset {{ path: {:?}, mime: {:?}, bytes: include_bytes!({:?}) }},\n",
            web_path,
            mime,
            abs.to_string_lossy()
        ));
    }
    code.push_str("];\n");

    fs::write(&dest, code).expect("write embedded_ui_assets.rs");
}
