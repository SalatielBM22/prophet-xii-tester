// SPDX-License-Identifier: GPL-3.0-only
use prophet_lv2::ttl::{generate_manifest_ttl, generate_plugin_ttl};
use std::path::PathBuf;

fn main() {
    let mut args = std::env::args().skip(1);
    let out_dir = PathBuf::from(args.next().unwrap_or_else(|| "target/lv2/prophet-xii.lv2".to_owned()));
    let bin_name = args.next().unwrap_or_else(|| "libprophet_lv2.so".to_owned());

    std::fs::create_dir_all(&out_dir).expect("create LV2 bundle directory");
    let manifest_path = out_dir.join("manifest.ttl");
    let plugin_path = out_dir.join("prophet-xii.ttl");

    std::fs::write(&manifest_path, generate_manifest_ttl(&bin_name))
        .expect("write manifest.ttl");
    std::fs::write(&plugin_path, generate_plugin_ttl(&bin_name))
        .expect("write prophet-xii.ttl");

    println!(
        "Generated LV2 bundle Turtle files in {} (binary: {})",
        out_dir.display(),
        bin_name
    );
}
