//! Offline callback benchmark; no device/loopback latency claim. Issue #1.
use prophet_params::ParamId::*;
use prophet_standalone::{AudioProcessor, Command};
use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
    sync::{atomic::AtomicU64, Arc},
    time::Instant,
};
struct Count;
thread_local! { static TRACK:Cell<bool>=const{Cell::new(false)}; static ALLOCS:Cell<usize>=const{Cell::new(0)}; }
unsafe impl GlobalAlloc for Count {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        TRACK.with(|t| {
            if t.get() {
                ALLOCS.with(|a| a.set(a.get() + 1))
            }
        });
        unsafe { System.alloc(l) }
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        TRACK.with(|t| {
            if t.get() {
                ALLOCS.with(|a| a.set(a.get() + 1))
            }
        });
        unsafe { System.alloc_zeroed(l) }
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        TRACK.with(|t| {
            if t.get() {
                ALLOCS.with(|a| a.set(a.get() + 1))
            }
        });
        unsafe { System.realloc(p, l, n) }
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        unsafe { System.dealloc(p, l) }
    }
}
#[global_allocator]
static ALLOCATOR: Count = Count;
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let value = |key: &str| {
        args.windows(2)
            .find(|v| v[0] == key)
            .and_then(|v| v[1].parse::<usize>().ok())
    };
    let count = value("--iterations").unwrap_or(100).max(10);
    let block = value("--block").unwrap_or(256).clamp(16, 2048);
    println!("sr,os,filter,voices,fx,frames,iterations,mean_us,p50_us,p95_us,p99_us,max_us,budget_us,misses,allocations,subnormals,audio_hash,kernel_inline_bytes,processor_inline_bytes");
    for sr in [48000., 96000.] {
        for os in [1., 2., 4.] {
            for morph in [0., 1.] {
                for voices in [0, 1, 6, 12] {
                    for fx in [0, 1, 2] {
                        let (mut tx, rx) = rtrb::RingBuffer::new(512);
                        let (tt, mut tr) = rtrb::RingBuffer::new(4);
                        let mut p = AudioProcessor::new(sr, rx, tt, Arc::new(AtomicU64::new(0)));
                        p.command(Command::Param {
                            id: OversamplingMode as usize,
                            value: os,
                        });
                        p.command(Command::Param {
                            id: FilterMorph as usize,
                            value: morph,
                        });
                        p.command(Command::Param {
                            id: ChorusMode as usize,
                            value: 0.,
                        });
                        p.command(Command::Param {
                            id: ReverbWet as usize,
                            value: if fx == 0 { 0. } else { 0.32 },
                        });
                        p.command(Command::Param {
                            id: ReverbEnsembleMode as usize,
                            value: if fx == 2 { 2. } else { 0. },
                        });
                        for n in 48..48 + voices {
                            tx.push(Command::Midi {
                                status: 0x90,
                                data1: n,
                                data2: 100,
                            })
                            .unwrap();
                        }
                        let mut capture_rx = if args.iter().any(|a| a == "--capture") {
                            let (ct, cr) = rtrb::RingBuffer::new(256);
                            p.attach_capture(ct);
                            Some(cr)
                        } else {
                            None
                        };
                        let mut out = vec![0f32; block * 2];
                        let mut times = Vec::with_capacity(count);
                        let mut hash = 0xcbf29ce484222325u64;
                        let mut subnormals = 0;
                        for _ in 0..32 {
                            p.process(&mut out, 2);
                            while tr.pop().is_ok() {}
                            if let Some(cr) = capture_rx.as_mut() {
                                while cr.pop().is_ok() {}
                            }
                        }
                        assert_eq!(
                            p.engine
                                .kernel
                                .last_telemetry
                                .active_voice_mask
                                .count_ones(),
                            voices as u32
                        );
                        ALLOCS.with(|a| a.set(0));
                        for _ in 0..count {
                            TRACK.with(|a| a.set(true));
                            let start = Instant::now();
                            p.process(&mut out, 2);
                            let us = start.elapsed().as_secs_f64() * 1e6;
                            TRACK.with(|a| a.set(false));
                            times.push(us);
                            for x in &out {
                                assert!(x.is_finite());
                                subnormals += usize::from(x.is_subnormal());
                                hash = (hash ^ x.to_bits() as u64).wrapping_mul(0x100000001b3);
                            }
                            while tr.pop().is_ok() {}
                            if let Some(cr) = capture_rx.as_mut() {
                                while cr.pop().is_ok() {}
                            }
                        }
                        let budget = block as f64 / sr as f64 * 1e6;
                        let mean = times.iter().sum::<f64>() / count as f64;
                        let misses = times.iter().filter(|t| **t > budget).count();
                        times.sort_by(f64::total_cmp);
                        let q = |p: f64| {
                            times[((count as f64 * p).ceil() as usize)
                                .saturating_sub(1)
                                .min(count - 1)]
                        };
                        println!("{sr},{os},{},{voices},{fx},{block},{count},{mean:.3},{:.3},{:.3},{:.3},{:.3},{budget:.3},{misses},{},{subnormals},{hash:016x},{},{}",if morph==0.{"curtis"}else{"moog"},q(0.5),q(0.95),q(0.99),q(1.),ALLOCS.with(Cell::get),std::mem::size_of::<prophet_dsp::ProphetXiiKernel>(),std::mem::size_of::<AudioProcessor>());
                    }
                }
            }
        }
    }
}
