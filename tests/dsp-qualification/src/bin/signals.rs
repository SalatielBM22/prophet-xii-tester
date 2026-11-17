// Worker-only measurements. Samples are little-endian f32, interleaved stereo for IR.
use prophet_analysis_core::validity::{measure, Validity};
use prophet_dsp::{
    fx::{
        reverb::{Algorithm, ReverbPort, ReverbSlot},
        solaris_reverb::{EnsembleMode, SolarisReverbParams},
    },
    vcf_moog::DualVcfStage,
};
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};
fn main() -> std::io::Result<()> {
    let dir = std::env::args_os()
        .nth(1)
        .expect("provide an output directory");
    let dir = Path::new(&dir);
    std::fs::create_dir_all(dir)?;
    let mut report = File::create(dir.join("vcf.csv"))?;
    writeln!(
        report,
        "rate,window,cutoff,resonance,drive,morph,validity,points,tail_rms,scaling_error"
    )?;
    for rate in [44100, 48000, 96000] {
        for n in [4096, 65536] {
            let m = measure(
                rate as f64,
                &[27.51, 100., 1000.],
                n,
                || {
                    let mut f = DualVcfStage::new(rate as f32, 1.);
                    f.set_params(27.51, 1., 1.1, 0.5, 0.35);
                    move |x| f.process_sample(x)
                },
                None,
            )
            .unwrap();
            assert_eq!(m.validity,Validity::LongTail);
            assert!(m.points.is_empty());
            writeln!(
                report,
                "{rate},{n},27.51,1,1.1,0.35,{:?},{},{},{}",
                m.validity,
                m.points.len(),
                m.tail_rms,
                m.relative_scaling_error
            )?;
        }
        for model in 0..2 {
            for drive in [1, 2, 4] {
                for tones in [1, 2, 3] {
                    let n = 8192;
                    let mut f = DualVcfStage::new(rate as f32, 1.);
                    f.set_params(
                        if tones == 3 { 0.4 * rate as f32 } else { 2000. },
                        0.2,
                        drive as f32,
                        0.5,
                        model as f32,
                    );
                    let mut file = BufWriter::new(File::create(
                        dir.join(format!("drive-{rate}-{model}-{drive}-{tones}.f32")),
                    )?);
                    for i in 0..2 * n {
                        let phase = std::f64::consts::TAU * i as f64 / n as f64;
                        let input = if tones == 3 {
                            0.8 * (1703. * phase).sin()
                        } else {
                            0.25 * ((64. * phase).sin()
                                + if tones == 2 { (107. * phase).sin() } else { 0. })
                        };
                        let y = f.process_sample(input as f32);
                        assert!(y.is_finite());
                        if i >= n {
                            file.write_all(&y.to_le_bytes())?;
                        }
                    }
                }
            }
        }
        for alg in [Algorithm::LegacyRC1, Algorithm::DiffuseV1] {
            let mut slot = ReverbSlot::prepare(rate as f32, alg).unwrap();
            let p = SolarisReverbParams {
                wet_mix: 1.,
                pre_delay_ms: 0.,
                decay_sec: 1.5,
                ensemble_mode: EnsembleMode::Forte,
                shimmer_amount: 0.7,
                ..Default::default()
            };
            let mut file = BufWriter::new(File::create(
                dir.join(format!("ir-{rate}-{}.f32", alg as u8)),
            )?);
            for i in 0..rate * 6 {
                let (l, r) = slot.process_stereo(if i == 0 { 0.5 } else { 0. }, 0., &p);
                assert!(l.is_finite() && r.is_finite());
                file.write_all(&l.to_le_bytes())?;
                file.write_all(&r.to_le_bytes())?;
            }
        }
    }
    Ok(())
}
