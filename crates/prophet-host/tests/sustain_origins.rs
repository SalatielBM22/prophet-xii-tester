// SPDX-License-Identifier: GPL-3.0-only
//! A physical damper pedal must sustain notes regardless of which origin played
//! them: a CASIO pedal (hardware MIDI) holds keys played on the on-screen keyboard.
use prophet_control::port::Prepare;
use prophet_host::{
    create_host_session, queue::PhysicalMidiSource, HostControlPlane, RtHostSlot, UiCommand,
};

struct Rig {
    rt: RtHostSlot,
    ctrl: HostControlPlane,
    l: Vec<f32>,
    r: Vec<f32>,
}
impl Rig {
    fn new() -> Self {
        let config = Prepare { sample_rate: 48_000, max_frames: 480, oversampling: 2 };
        let (rt, mut ctrl) = create_host_session("test", vec![], config, true).unwrap();
        ctrl.set_audio_running(true, None, None);
        Self { rt, ctrl, l: vec![0.0; 480], r: vec![0.0; 480] }
    }
    fn blocks(&mut self, n: usize) {
        for _ in 0..n {
            self.rt.process_planar_f32(&mut self.l, &mut self.r, &[], None, None, None).unwrap();
            self.ctrl.poll();
        }
    }
    fn ui(&mut self, status: u8, key: u8, velocity: u8) {
        self.ctrl.dispatch(UiCommand::Midi { status, data1: key, data2: velocity, note_id: Some(u32::from(key)) });
    }
    fn hw(&mut self, status: u8, d1: u8, d2: u8) {
        self.ctrl.dispatch_hardware_midi(PhysicalMidiSource::Native, status, d1, d2, None, 0).unwrap();
    }
    fn gated(&mut self) -> usize {
        self.ctrl.snapshot().engine_port.voices.iter().filter(|v| v.gate).count()
    }
    fn active(&mut self) -> u32 {
        self.ctrl.snapshot().kernel_telemetry.active_voice_mask.count_ones()
    }
}

#[test]
fn hardware_pedal_sustains_twelve_local_ui_notes() {
    let mut rig = Rig::new();
    rig.hw(0xb0, 64, 127);
    rig.blocks(2);
    for k in 0..12u8 {
        rig.ui(0x90, 48 + 2 * k, 100);
        rig.blocks(3);
        rig.ui(0x80, 48 + 2 * k, 0);
        // Longer than the default amp release, so an unsustained voice would go idle.
        rig.blocks(20);
    }
    assert_eq!(rig.gated(), 12, "every released UI key stays gated under the pedal");
    assert_eq!(rig.active(), 12);

    rig.hw(0xb0, 64, 0);
    rig.blocks(2);
    assert_eq!(rig.gated(), 0, "pedal up releases notes of every origin");
}

#[test]
fn local_pedal_and_hardware_pedal_combine_per_channel() {
    let mut rig = Rig::new();
    rig.ui(0xb0, 64, 127);
    rig.hw(0xb0, 64, 127);
    rig.hw(0x90, 60, 100);
    rig.blocks(2);
    rig.hw(0x80, 60, 0);
    rig.ui(0xb0, 64, 0);
    rig.blocks(2);
    assert_eq!(rig.gated(), 1, "the hardware pedal still holds the note");
    rig.hw(0xb0, 64, 0);
    rig.blocks(2);
    assert_eq!(rig.gated(), 0);
}

#[test]
fn panic_of_pedal_origin_releases_notes_it_was_holding() {
    let mut rig = Rig::new();
    rig.hw(0xb0, 64, 127);
    rig.ui(0x90, 60, 100);
    rig.blocks(2);
    rig.ui(0x80, 60, 0);
    rig.blocks(2);
    assert_eq!(rig.gated(), 1);
    // Disconnecting the controller clears its pedal; the UI note must not hang.
    rig.hw(0xb0, 123, 0);
    rig.blocks(2);
    assert_eq!(rig.gated(), 0);
}
