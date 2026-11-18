// SPDX-License-Identifier: GPL-3.0-only
//! Off-thread engine preparation, atomic audio-thread swap (zero deallocation on RT thread),
//! and bounded host runtime utilities.

use crate::{
    preset::{self, HostPresetError},
    queue::{BlockScheduler, BlockSummary, MusicTransport, SerialIssuer},
    wire::SnapshotPod,
};
use prophet_control::{
    music::{Arpeggiator, NoteScheduler, PatternBank},
    port::{Ack, Command, EnginePort, PortError, Prepare, Status},
    state::SoundState,
    PreparedEngine,
};
use rtrb::{Consumer, Producer, RingBuffer};

pub struct PreparedSwap {
    pub swap_id: u64,
    pub engine: Box<PreparedEngine>,
    pub bank: Option<Box<PatternBank>>,
    pub performance: Option<(NoteScheduler, Arpeggiator)>,
}

pub enum RetiredResource {
    Engine(Box<PreparedEngine>),
    Bank(Box<PatternBank>),
}

/// Non-RT side handle that prepares replacement instances and frees retired instances safely
/// outside the audio callback.
pub struct WorkerSwapCoordinator {
    swap_tx: Producer<PreparedSwap>,
    retire_rx: Consumer<RetiredResource>,
    next_swap_id: u64,
}

/// Audio-thread side handle storing the active `PreparedEngine`, zero-allocation `BlockScheduler`,
/// optional `MusicTransport`, and lock-free swap/retire channels.
pub struct AudioEngineSlot {
    pub engine: Box<PreparedEngine>,
    pub scheduler: BlockScheduler,
    pub music: Option<MusicTransport>,
    swap_rx: Consumer<PreparedSwap>,
    retire_tx: Producer<RetiredResource>,
    scope: [f32; 1024],
    scope_pos: usize,
    active_swap_id: u64,
    deferred_swaps: u64,
}

pub fn create_swap_pair(
    initial: PreparedEngine,
    with_music: bool,
    queue_capacity: usize,
) -> (AudioEngineSlot, WorkerSwapCoordinator) {
    let cap = queue_capacity.max(2);
    let (swap_tx, swap_rx) = RingBuffer::new(cap);
    let (retire_tx, retire_rx) = RingBuffer::new(cap * 2);
    let sr = initial.configuration().sample_rate;
    let music = if with_music {
        MusicTransport::new(sr).ok()
    } else {
        None
    };
    let slot = AudioEngineSlot {
        engine: Box::new(initial),
        scheduler: BlockScheduler::new(),
        music,
        swap_rx,
        retire_tx,
        scope: [0.0; 1024],
        scope_pos: 0,
        active_swap_id: 0,
        deferred_swaps: 0,
    };
    let worker = WorkerSwapCoordinator {
        swap_tx,
        retire_rx,
        next_swap_id: 1,
    };
    (slot, worker)
}

impl WorkerSwapCoordinator {
    pub fn collect_retired(&mut self) -> usize {
        let mut freed = 0usize;
        while let Ok(item) = self.retire_rx.pop() {
            drop(item);
            freed += 1;
        }
        freed
    }

    pub fn prepare_reconfig(
        &mut self,
        state: &SoundState,
        sample_rate: u32,
        max_frames: u16,
    ) -> Result<u64, Status> {
        self.collect_retired();
        let engine = state.prepare(sample_rate, max_frames)?;
        self.submit_prepared(engine, None, None)
    }

    pub fn prepare_raw(
        &mut self,
        config: Prepare,
    ) -> Result<u64, PortError> {
        self.collect_retired();
        let engine = PreparedEngine::prepare(config)?;
        self.submit_prepared(engine, None, None)
            .map_err(|_| PortError::Capacity)
    }

    pub fn prepare_preset_bytes(
        &mut self,
        bytes: &[u8],
        sample_rate: u32,
        max_frames: u16,
    ) -> Result<u64, HostPresetError> {
        self.collect_retired();
        let bundle = preset::decode_and_prepare(bytes, sample_rate, max_frames)?;
        self.submit_prepared(bundle.engine, bundle.bank, bundle.performance)
            .map_err(HostPresetError::PrepareFailed)
    }

    pub fn submit_prepared(
        &mut self,
        engine: PreparedEngine,
        bank: Option<Box<PatternBank>>,
        performance: Option<(NoteScheduler, Arpeggiator)>,
    ) -> Result<u64, Status> {
        self.collect_retired();
        let id = self.next_swap_id;
        self.next_swap_id = self.next_swap_id.wrapping_add(1).max(1);
        let msg = PreparedSwap {
            swap_id: id,
            engine: Box::new(engine),
            bank,
            performance,
        };
        self.swap_tx.push(msg).map_err(|_| Status::Capacity)?;
        Ok(id)
    }
}

impl AudioEngineSlot {
    pub fn active_swap_id(&self) -> u64 {
        self.active_swap_id
    }

    pub fn deferred_swaps(&self) -> u64 {
        self.deferred_swaps
    }

    /// Checks whether a newly prepared replacement is waiting and installs it ONLY when the
    /// retire queue has capacity (so the audio thread never deallocates `Box<PreparedEngine>`!).
    /// If `force` is true (stopped transport boundary) it swaps even with voices active; otherwise
    /// it swaps immediately without dropping an already-retired engine.
    pub fn poll_swap(&mut self, serials: Option<&SerialIssuer>) -> bool {
        if self.retire_tx.slots() < 2 || self.swap_rx.is_empty() {
            if !self.swap_rx.is_empty() {
                self.deferred_swaps += 1;
            }
            return false;
        }
        let mut applied = false;
        while self.retire_tx.slots() >= 2 {
            let Ok(next) = self.swap_rx.pop() else {
                break;
            };
            // Flush stop release cleanly on outgoing engine before retiring it if needed.
            if let Some(m) = self.music.as_mut() {
                let _ = m.flush_stop_before_suspend(&mut self.engine);
            }
            let old = core::mem::replace(&mut self.engine, next.engine);
            let _ = self.retire_tx.push(RetiredResource::Engine(old));

            if let Some(new_bank) = next.bank {
                if let Some(m) = self.music.as_mut() {
                    let old_bank = core::mem::replace(&mut m.bank, new_bank);
                    let _ = self.retire_tx.push(RetiredResource::Bank(old_bank));
                }
            }
            if let Some((sched, arp)) = next.performance {
                if let Some(m) = self.music.as_mut() {
                    m.scheduler = sched;
                    m.arp = arp;
                }
            }
            self.scheduler.clear_carried();
            if let Some(s) = serials {
                s.advance_epoch_and_reset();
            }
            self.active_swap_id = next.swap_id;
            applied = true;
        }
        applied
    }

    #[allow(clippy::too_many_arguments)]
    pub fn process_block<F, M>(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        incoming: &[Command],
        ack_sink: F,
        music_status_sink: M,
    ) -> Result<BlockSummary, PortError>
    where
        F: FnMut(Ack),
        M: FnMut(Status),
    {
        let summary = self.scheduler.process_block(
            &mut self.engine,
            left,
            right,
            incoming,
            ack_sink,
            self.music.as_mut(),
            music_status_sink,
        )?;
        for &sample in left.iter() {
            self.scope[self.scope_pos] = sample;
            self.scope_pos = (self.scope_pos + 1) % 1024;
        }
        Ok(summary)
    }

    pub fn capture_snapshot(&self, xruns: Option<u32>, cpu_percent: Option<f32>) -> SnapshotPod {
        let mut linear_scope = [0.0f32; 1024];
        for (i, dst) in linear_scope.iter_mut().enumerate() {
            *dst = self.scope[(self.scope_pos + i) % 1024];
        }
        let (pat, playing) = self
            .music
            .as_ref()
            .map_or((0, false), |m| (m.active_pattern as u8, m.enabled));
        SnapshotPod::from_engine(
            &self.engine,
            linear_scope,
            xruns,
            cpu_percent,
            pat,
            playing,
            0,
        )
    }
}
