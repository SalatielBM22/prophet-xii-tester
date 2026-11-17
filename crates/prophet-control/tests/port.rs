use prophet_control::{port::*, PreparedEngine};

fn midi(origin: u8, serial: u32, status: u8, key: u8, value: u8, id: u32) -> Command {
    Command {
        offset: 0,
        origin,
        serial,
        action: Action::Midi {
            status,
            data1: key,
            data2: value,
            note_id: id,
        },
    }
}
fn engine() -> PreparedEngine {
    PreparedEngine::prepare(Prepare {
        sample_rate: 48000,
        max_frames: 480,
        oversampling: 2,
    })
    .unwrap()
}
fn send(p: &mut PreparedEngine, cmds: &[Command]) -> Vec<Status> {
    let mut a = vec![Ack::default(); cmds.len()];
    p.process(&mut [0.; 64], &mut [0.; 64], cmds, &mut a)
        .unwrap();
    a.iter().map(|v| v.status).collect()
}
#[test]
fn overlap_replay_and_scoped_pedals() {
    let mut p = engine();
    let a = midi(1, 1, 0x91, 64, 120, 55);
    assert_eq!(
        send(&mut p, &[a, a, midi(2, 1, 0x91, 64, 90, 2)]),
        [Status::Applied, Status::Duplicate, Status::Applied]
    );
    assert_eq!(p.notes().count(), 2);
    assert_eq!(p.telemetry().active_voice_mask.count_ones(), 2);
    send(
        &mut p,
        &[
            midi(1, 2, 0xb1, 66, 127, 0),
            midi(1, 3, 0x91, 64, 0, 55),
            midi(2, 2, 0x81, 64, 0, 2),
        ],
    );
    assert_eq!(p.notes().filter(|n| !n.pressed && n.sostenuto).count(), 1);
    send(&mut p, &[midi(1, 4, 0xb1, 66, 0, 0)]);
    assert_eq!(p.notes().count(), 0);
}
#[test]
fn invalid_block_is_atomic_and_glide_is_independent() {
    let mut p = engine();
    let mut e = midi(2, 1, 0x90, 64, 100, 2);
    e.offset = 64;
    let mut l = [3.; 64];
    assert_eq!(
        p.process(&mut l, &mut [0.; 64], &[e], &mut [Ack::default()]),
        Err(PortError::Offset)
    );
    assert_eq!(p.rendered_frames(), 0);
    assert_eq!(l, [3.; 64]);
    assert_eq!(p.notes().count(), 0);
    let param = |serial, id, value| Command {
        offset: 0,
        origin: 0,
        serial,
        action: Action::Parameter { id, value },
    };
    assert_eq!(
        send(
            &mut p,
            &[param(1, 42, 0.3), param(2, 83, 0.0), param(3, 78, 1.0)]
        ),
        [Status::Applied; 3]
    );
    assert_eq!(p.parameter(42), Some(0.3));
    assert_eq!(p.parameter(83), Some(0.));
    assert_eq!(
        send(&mut p, &[param(4, 23, f32::NAN), param(5, 63, 4.)]),
        [Status::Invalid, Status::Unsupported]
    );
}
