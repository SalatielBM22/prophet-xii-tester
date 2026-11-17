// Run with one process/thread. Count alloc/free around process, including note onset.
use prophet_control::{port::*, PreparedEngine};
use prophet_dsp::vector::{Patch, Topology};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicUsize, Ordering::Relaxed},
    time::Instant,
};
struct Count;
static CALLS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Count {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        CALLS.fetch_add(1, Relaxed);
        BYTES.fetch_add(l.size(), Relaxed);
        System.alloc(l)
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        CALLS.fetch_add(1, Relaxed);
        BYTES.fetch_sub(l.size(), Relaxed);
        System.dealloc(p, l)
    }
}
#[global_allocator]
static GLOBAL: Count = Count;
fn main() {
    println!("rate,oversampling,voices,topology,block,warmup,blocks,p50_us,p95_us,p99_us,max_us,budget_us,misses,alloc_free,prepared_heap_bytes");
    for rate in [44100, 48000, 96000] {
        for os in [1, 2, 4] {
            for voices in [1, 6, 12] {
                for topology in [
                    Topology::Classic2,
                    Topology::Extended3,
                    Topology::Extended4,
                    Topology::Vector4,
                ] {
                    let memory_before = BYTES.load(Relaxed);
                    let mut p = PreparedEngine::prepare(Prepare {
                        sample_rate: rate,
                        max_frames: 480,
                        oversampling: os,
                    })
                    .unwrap();
                    p.set_patch(Patch {
                        topology,
                        ..Default::default()
                    })
                    .unwrap();
                    let heap = BYTES.load(Relaxed) - memory_before;
                    let notes: [Command; 12] = std::array::from_fn(|i| Command {
                        offset: 0,
                        origin: 1,
                        serial: i as u32 + 1,
                        action: Action::Midi {
                            status: 0x90,
                            data1: 48 + i as u8,
                            data2: 100,
                            note_id: i as u32,
                        },
                    });
                    let mut ack = [Ack::default(); 12];
                    let mut l = [0.; 480];
                    let mut r = l;
                    let mut us = [0.; 128];
                    let mut allocations = 0;
                    for i in 0..144 {
                        let stamp = Instant::now();
                        let before = CALLS.load(Relaxed);
                        p.process(
                            &mut l,
                            &mut r,
                            if i == 0 { &notes[..voices] } else { &[] },
                            &mut ack,
                        )
                        .unwrap();
                        allocations += CALLS.load(Relaxed) - before;
                        let elapsed = stamp.elapsed().as_secs_f64() * 1e6;
                        if i >= 16 {
                            us[i - 16] = elapsed;
                        }
                        assert!(l.iter().chain(&r).all(|x| x.is_finite()));
                        std::hint::black_box(&l);
                    }
                    assert_eq!(allocations, 0);
                    let budget = 480e6 / f64::from(rate);
                    let misses = us.iter().filter(|v| **v > budget).count();
                    us.sort_unstable_by(f64::total_cmp);
                    println!("{rate},{os},{voices},{topology:?},480,16,128,{:.3},{:.3},{:.3},{:.3},{budget:.3},{misses},{allocations},{heap}",us[63],us[121],us[126],us[127]);
                }
            }
        }
    }
}
