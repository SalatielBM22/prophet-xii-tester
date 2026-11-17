// Compare against an identical, unedited stack at the first frame of the edit.
// Later waveform divergence is expected as detune/pan/voice count change.
use prophet_control::{ipc_bridge::ParamId::*, ProphetXiiPlugin};
fn setup(rate: f32, factor: u8, count: u8) -> ProphetXiiPlugin {
    let mut p = ProphetXiiPlugin::new(rate);
    let b = &p.ipc.params;
    for (id, value) in [
        (ReverbWet, 0.),
        (ChorusMode, 0.),
        (OversamplingMode, factor as f32),
        (UnisonEnabled, 1.),
        (UnisonVoices, count as f32),
    ] {
        b.set(id, value);
    }
    p.sync_parameters();
    p.handle_midi_message(0x91, 62, 100);
    p.handle_midi_message(0xb1, 64, 127);
    p.handle_midi_message(0x81, 62, 0);
    p
}
fn main() {
    println!(
        "rate,os,from,to,seam_delta_fs,control_delta_fs,first_edit_error_fs,peak_delta_20ms_fs"
    );
    for rate in [44100., 48000., 96000.] {
        for os in [1, 2, 4] {
            for (start, end) in [(1, 12), (12, 2), (6, 6)] {
                let (mut a, mut b) = (setup(rate, os, start), setup(rate, os, start));
                let (mut al, mut ar, mut bl, mut br) = ([0.; 257], [0.; 257], [0.; 257], [0.; 257]);
                for _ in 0..31 {
                    a.process_audio_block(&mut al, &mut ar);
                    b.process_audio_block(&mut bl, &mut br);
                }
                assert_eq!(al, bl);
                let mut previous = [al[256], ar[256]];
                b.ipc.params.set(UnisonVoices, end as f32);
                b.ipc.params.set(UnisonDetune, 40.);
                b.ipc.params.set(UnisonSpread, 0.1);
                let (mut seam, mut control, mut excess, mut peak) = (0f32, 0f32, 0f32, 0f32);
                let frames = (rate * 0.020) as usize;
                for pos in (0..frames).step_by(257) {
                    let n = (frames - pos).min(257);
                    a.process_audio_block(&mut al[..n], &mut ar[..n]);
                    b.process_audio_block(&mut bl[..n], &mut br[..n]);
                    for i in 0..n {
                        let dy = (bl[i] - previous[0]).abs().max((br[i] - previous[1]).abs());
                        if pos + i == 0 {
                            seam = dy;
                            control = (al[i] - previous[0]).abs().max((ar[i] - previous[1]).abs());
                            excess = (al[i] - bl[i]).abs().max((ar[i] - br[i]).abs());
                        }
                        peak = peak.max(dy);
                        previous = [bl[i], br[i]];
                    }
                }
                println!("{rate},{os},{start},{end},{seam},{control},{excess},{peak}");
            }
        }
    }
}
