use prophet_control::{port::*, PreparedEngine};
use prophet_dsp::vector::*;

fn render(topology: Topology, route: Route) -> Vec<f32> {
    let mut engine = PreparedEngine::prepare(Prepare {
        sample_rate: 48000,
        max_frames: 480,
        oversampling: 2,
    })
    .unwrap();
    engine
        .set_patch(Patch {
            topology,
            routes: [route; 2],
            ..Default::default()
        })
        .unwrap();
    let mut out = Vec::new();
    let mut events = &[
        Command {
            origin: 1,
            serial: 1,
            offset: 7,
            action: Action::Midi {
                status: 0x90,
                data1: 60,
                data2: 120,
                note_id: 1,
            },
        },
        Command {
            origin: 1,
            serial: 2,
            offset: 11,
            action: Action::VectorXY { x: 1., y: 0. },
        },
        Command {
            origin: 1,
            serial: 3,
            offset: 12,
            action: Action::VectorXY { x: 1., y: 1. },
        },
    ][..];
    for _ in 0..8 {
        let mut acks = [Ack::default(); 3];
        let mut l = [0.; 480];
        engine
            .process(&mut l, &mut [0.; 480], events, &mut acks)
            .unwrap();
        if !events.is_empty() && topology == Topology::Vector4 {
            assert_eq!(acks[1].status, Status::Applied);
            assert_eq!(acks[2].status, Status::Invalid);
        }
        out.extend(l);
        events = &[];
    }
    assert!(out.iter().all(|x| x.is_finite()));
    assert!(out.iter().any(|x| x.abs() > 1e-3));
    out
}
#[test]
fn audible_and_distinct_topologies_and_forward_routes() {
    let classic = render(Topology::Classic2, Route::Sum);
    let three = render(Topology::Extended3, Route::Sum);
    let four = render(Topology::Extended4, Route::Sum);
    assert_ne!(classic, three);
    assert_ne!(three, four);
    let mut seen = vec![four];
    for route in [
        Route::Sum,
        Route::Frequency,
        Route::Phase,
        Route::Sync,
        Route::Ring,
    ] {
        let v = render(Topology::Vector4, route);
        assert!(!seen.contains(&v));
        seen.push(v);
    }
}
#[test]
fn diamond_geometry_and_voice_trajectory() {
    for i in -32i32..=32 {
        for j in -32i32..=32 {
            let p = Point {
                x: i as f32 / 32.,
                y: j as f32 / 32.,
            };
            if p.valid() {
                let w = p.weights();
                assert!(w.iter().all(|&v| v >= 0.));
                assert_eq!(w.iter().sum::<f32>(), 1.);
            }
        }
    }
    for (i, (x, y)) in [(-1., 0.), (0., 1.), (1., 0.), (0., -1.)]
        .into_iter()
        .enumerate()
    {
        assert_eq!(Point { x, y }.weights()[i], 1.);
    }
    let mut patch = Patch::default();
    patch.stages = [Stage {
        millis: 1,
        target: Point::default(),
    }; 5];
    patch.stages[3].target.x = 1.;
    patch.stages[4].target.y = -1.;
    let mut t = Trajectory::default();
    for _ in 0..32 {
        t.advance(&patch, 8000.);
    }
    assert_eq!(t.advance(&patch, 8000.), [0., 0., 1., 0.]);
    t.release();
    for _ in 0..8 {
        t.advance(&patch, 8000.);
    }
    assert_eq!(t.advance(&patch, 8000.), [0., 0., 0., 1.]);
}

#[test]
fn per_note_pressure_moves_only_its_trajectory() {
    let mut e = PreparedEngine::prepare(Prepare {
        sample_rate: 48000,
        max_frames: 480,
        oversampling: 1,
    })
    .unwrap();
    let mut patch = Patch {
        topology: Topology::Vector4,
        ..Default::default()
    };
    patch.modulation[1].x = 1.;
    e.set_patch(patch).unwrap();
    let mut l = [0.; 480];
    let mut r = l;
    let mut events = vec![];
    for (i, (status, key, vel)) in [(0x91, 60, 90), (0x92, 65, 90), (0xd1, 127, 0)]
        .into_iter()
        .enumerate()
    {
        events.push(Command {
            origin: 1,
            serial: i as u32 + 1,
            offset: 0,
            action: Action::Midi {
                status,
                data1: key,
                data2: vel,
                note_id: key as u32,
            },
        });
    }
    for _ in 0..5 {
        e.process(&mut l, &mut r, &events, &mut [Ack::default(); 3])
            .unwrap();
        events.clear();
    }
    let weights = e.vector_weights();
    assert!(weights[0].unwrap()[2] > 0.999);
    assert_eq!(weights[1], Some([0.25; 4]));
}
