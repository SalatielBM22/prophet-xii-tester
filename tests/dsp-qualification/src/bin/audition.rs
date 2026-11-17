// Render the same chord phrase twice, switching only the prepared tank.
use prophet_control::{port::*, PreparedEngine};
use prophet_dsp::fx::reverb::Algorithm;
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::PathBuf,
};
fn main() -> std::io::Result<()> {
    let dir = PathBuf::from(std::env::args_os().nth(1).expect("output directory"));
    std::fs::create_dir_all(&dir)?;
    for alg in [Algorithm::LegacyRC1, Algorithm::DiffuseV1] {
        let mut p = PreparedEngine::prepare(Prepare {
            sample_rate: 48000,
            max_frames: 480,
            oversampling: 2,
        })
        .unwrap();
        p.select_reverb(alg).unwrap();
        let mut out = BufWriter::new(File::create(
            dir.join(format!("audition-{}.f32", alg as u8)),
        )?);
        let mut serial = 0u32;
        for block in 0..700 {
            let mut commands = Vec::new();
            let mut submit = |action| {
                serial += 1;
                commands.push(Command {
                    offset: 0,
                    origin: 1,
                    serial,
                    action,
                });
            };
            if block == 0 {
                for (id, value) in [(51, 2.), (52, 0.7), (53, 2.5), (54, 0.7), (62, 0.65)] {
                    submit(Action::Parameter { id, value });
                }
            }
            if [0, 75, 150].contains(&block) {
                let offset = (block / 75) * 2;
                for k in [48, 55, 64] {
                    let key = (k + offset) as u8;
                    submit(Action::Midi {
                        status: 0x90,
                        data1: key,
                        data2: 96,
                        note_id: u32::from(key),
                    });
                }
            }
            if [60, 135, 225].contains(&block) {
                submit(Action::PanicOrigin);
            }
            let (mut l, mut r) = ([0.; 480], [0.; 480]);
            let mut acks = [Ack::default(); 8];
            p.process(&mut l, &mut r, &commands, &mut acks).unwrap();
            assert!(acks[..commands.len()]
                .iter()
                .all(|a| a.status == Status::Applied));
            for (l, r) in l.into_iter().zip(r) {
                out.write_all(&l.to_le_bytes())?;
                out.write_all(&r.to_le_bytes())?;
            }
        }
    }
    Ok(())
}
