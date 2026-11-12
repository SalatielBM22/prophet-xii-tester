use prophet_analysis::*;
use std::{
    alloc::{GlobalAlloc, Layout, System},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
struct Counter;
static ENABLED: AtomicBool = AtomicBool::new(false);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
unsafe impl GlobalAlloc for Counter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if ENABLED.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        System.alloc(layout)
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        System.dealloc(p, layout)
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        if ENABLED.load(Ordering::Relaxed) {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        System.realloc(p, l, n)
    }
}
#[global_allocator]
static ALLOCATOR: Counter = Counter;
#[test]
fn no_allocations_after_planning_in_spectrum_stft_transfer() {
    let c = Config::default();
    let mut a = SpectrumAnalyzer::new(c).unwrap();
    let x = vec![0.25; 4096];
    let mut s = Stft::new(c, 1024).unwrap();
    let mut t = TransferAnalyzer::new(8192).unwrap();
    #[cfg(feature = "web")]
    let mut web = WebAnalyzer::new(4096, 48000.0, 0, 1.0, 25, 1.0, 8.6).unwrap();
    let impulse = vec![0.0; 4096];
    let mut points = vec![TransferPoint::default(); 256];
    let mut f = vec![0.0; 256];
    log_frequencies(48000.0, 20.0, 20000.0, &mut f).unwrap();
    a.analyze(&x).unwrap();
    t.analyze(&impulse, 48000.0, &f, -100.0, &mut points, None)
        .unwrap();
    ALLOCS.store(0, Ordering::SeqCst);
    ENABLED.store(true, Ordering::SeqCst);
    for i in 0..10 {
        std::hint::black_box(a.analyze(&x).unwrap());
        s.push(i, i * 4096, &x, |m, v| {
            std::hint::black_box((m, v));
        })
        .unwrap();
        t.analyze(&impulse, 48000.0, &f, -100.0, &mut points, None)
            .unwrap();
    }
    #[cfg(feature = "web")]
    for sequence in 0..128 {
        assert!(web.push(sequence as f64, (sequence * 64) as f64, 64) >= 0);
        assert_eq!(web.push(-1.0, 0.0, 64), -2);
        assert_eq!(web.push(0.0, 0.0, 65), -1);
    }
    ENABLED.store(false, Ordering::SeqCst);
    assert_eq!(ALLOCS.load(Ordering::SeqCst), 0);
}
