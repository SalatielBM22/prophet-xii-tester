// Original Fourier-series waveforms. Generate once at build time so every voice
// shares immutable, harmonically limited tables with no lazy initialization.
use std::{
    env,
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};
fn main() -> std::io::Result<()> {
    println!("cargo:rerun-if-changed=build.rs");
    let mut out = BufWriter::new(File::create(
        Path::new(&env::var_os("OUT_DIR").unwrap()).join("waves.rs"),
    )?);
    writeln!(out, "static BANK: [[[f32; 513]; 6]; 2] = [")?;
    for family in 0..2 {
        writeln!(out, "[")?;
        for level in 0..6 {
            writeln!(out, "[")?;
            for i in 0..=512 {
                let theta = std::f64::consts::TAU * (i % 512) as f64 / 512.;
                let sample = (1..=1 << level)
                    .map(|k| {
                        let amplitude = if family == 0 {
                            1. / k as f64
                        } else if k % 2 == 1 {
                            1. / (k * k) as f64
                        } else {
                            0.
                        };
                        amplitude * (k as f64 * theta).sin()
                    })
                    .sum::<f64>()
                    * 0.55;
                writeln!(out, "{:?},", sample as f32)?;
            }
            writeln!(out, "],")?;
        }
        writeln!(out, "],")?;
    }
    writeln!(out, "];")
}
