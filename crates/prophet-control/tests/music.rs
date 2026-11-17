use prophet_control::{music::*, port::*, PreparedEngine};

fn trace(rate: u32, sizes: &[u16], arp: Option<&Arpeggiator>) -> Vec<(u64, Trigger)> {
    let mut p = Pattern::default();
    p.len = 4;
    p.steps[0] = Step {
        kind: Kind::Note,
        key: 60,
        velocity: 120,
        ratchet: 3,
        accent: true,
        glide: true,
        ..Default::default()
    };
    p.steps[1].kind = Kind::Tie;
    p.steps[3] = Step {
        kind: Kind::Note,
        key: 65,
        ..Default::default()
    };
    let mut t = NoteScheduler::new(rate, 123456, 30).unwrap();
    t.seek(0).unwrap();
    let mut buf = [MusicEvent::default(); 64];
    let mut result = vec![];
    let mut time = 0;
    for &size in sizes.iter().cycle() {
        let size = size.min((rate * 10 - time).min(1024) as u16);
        if size == 0 {
            break;
        }
        let n = t.block(size, &p, arp, &mut buf).unwrap();
        result.extend(
            buf[..n]
                .iter()
                .map(|e| (u64::from(time) + u64::from(e.offset), e.trigger)),
        );
        time += u32::from(size);
    }
    result
}
#[test]
fn independent_of_block_partition_with_ratcheted_swing() {
    for rate in [44100, 48000, 96000] {
        let a = trace(rate, &[480], None);
        assert_eq!(a, trace(rate, &[17, 511, 1024, 65], None));
        let attacks: Vec<_> = a
            .iter()
            .filter(|e| matches!(e.1, Trigger::Note { .. }))
            .collect();
        let first_step = rate as u64 * 19500 / 123456;
        assert_eq!(attacks[1].0, first_step / 3);
        assert_eq!(attacks[3].0, rate as u64 * 49500 / 123456);
        assert!(matches!(
            attacks[0].1,
            Trigger::Note {
                velocity: 127,
                glide: true,
                ..
            }
        ));
    }
}
#[test]
fn arp_order_and_seed_and_capacity_retry() {
    let mut a = Arpeggiator::default();
    for k in [67, 60, 64] {
        assert!(a.key(k, true));
    }
    for (mode, first) in [
        (Order::Up, 60),
        (Order::Down, 67),
        (Order::AsPlayed, 67),
        (Order::UpDown, 60),
    ] {
        a.order = mode;
        assert!(matches!(trace(44100,&[480],Some(&a))[2].1,Trigger::Note{key,..} if key==first));
    }
    a.order = Order::Random;
    assert_eq!(
        trace(48000, &[64], Some(&a)),
        trace(48000, &[480], Some(&a))
    );
    a.order = Order::Chord;
    let mut t = NoteScheduler::new(48000, 120000, 0).unwrap();
    t.seek(0).unwrap();
    let mut p = Pattern::default();
    p.steps[0].kind = Kind::Note;
    assert_eq!(
        t.block(480, &p, Some(&a), &mut []),
        Err(MusicError::Capacity)
    );
    let mut buf = [MusicEvent::default(); 16];
    let n = t.block(480, &p, Some(&a), &mut buf).unwrap();
    assert_eq!(n, 5);
    let mut e = PreparedEngine::prepare(Prepare {
        sample_rate: 48000,
        max_frames: 480,
        oversampling: 1,
    })
    .unwrap();
    let host = [Command {
        origin: 1,
        serial: 1,
        offset: 0,
        action: Action::Midi {
            status: 0x90,
            data1: 60,
            data2: 100,
            note_id: 9,
        },
    }];
    let mut result = [Status::Invalid; 16];
    let mut l = [0.; 480];
    let mut r = l;
    e.process_music(
        &mut l,
        &mut r,
        &host,
        &mut [Ack::default()],
        2,
        &buf[..n],
        &mut result,
    )
    .unwrap();
    assert_eq!(e.notes().count(), 4);
    t.stop();
    let n = t.block(480, &p, None, &mut buf).unwrap();
    e.process_music(&mut l, &mut r, &[], &mut [], 2, &buf[..n], &mut result)
        .unwrap();
    assert_eq!(e.notes().count(), 1);
    assert_eq!(e.notes().next().unwrap().origin, 1);
}

#[test]
fn loop_releases_previous_origin_and_tempo_change_preserves_downbeat() {
    let mut t = NoteScheduler::new(8000, 120000, 0).unwrap();
    let mut p = Pattern::default();
    for (i, s) in p.steps.iter_mut().enumerate() {
        s.kind = Kind::Note;
        s.key = 40 + i as u8;
    }
    let mut buf = [MusicEvent::default(); 32];
    t.start_grid(2, 500).unwrap();
    let n = t.block(500, &p, None, &mut buf).unwrap();
    assert!(matches!(buf[n - 1].trigger, Trigger::Note { key: 42, .. }));
    t.queue_tempo(240000).unwrap();
    let n = t.block(1000, &p, None, &mut buf).unwrap();
    assert!(buf[..n]
        .iter()
        .any(|e| e.offset == 500 && matches!(e.trigger, Trigger::Note { key: 43, .. })));
    let n = t.block(1, &p, None, &mut buf).unwrap();
    assert!(matches!(buf[n - 1].trigger, Trigger::Note { key: 44, .. }));
    t.start_grid(2, 2001).unwrap();
    let n = t.block(1, &p, None, &mut buf).unwrap();
    assert_eq!(buf[0].trigger, Trigger::Release);
    assert!(matches!(buf[n - 1].trigger, Trigger::Note { key: 42, .. }));
}
