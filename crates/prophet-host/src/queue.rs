// SPDX-License-Identifier: GPL-3.0-only
//! Real-time safe command ordering, right-endpoint carry, segment splitting (`<= max_frames`,
//! `<= 256` combined events), canonical physical MIDI arbitration, MIDI CC Learn routing, and
//! transactional `NoteScheduler` / `Arpeggiator` execution via `PreparedEngine`.

use core::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use prophet_control::{
    music::{Arpeggiator, MusicError, MusicEvent, NoteScheduler, PatternBank},
    port::{Ack, Action, Command, EnginePort, PortError, Status, MAX_EVENTS, ORIGINS},
    PreparedEngine,
};
use prophet_params::{AtomicParamBank, ParamId, NUM_PARAMS, PARAM_SPECS};
use serde::{Deserialize, Serialize};

pub const ORIGIN_DAW_HOST: u8 = 0;
pub const ORIGIN_HARDWARE_MIDI: u8 = 1;
pub const ORIGIN_LOCAL_UI: u8 = 2;
pub const ORIGIN_REMOTE_WS: u8 = 3;
pub const ORIGIN_WEB_MIDI: u8 = 4;
pub const ORIGIN_USB_MIDI: u8 = 5;
pub const ORIGIN_TEST_HARNESS: u8 = 6;
pub const ORIGIN_MUSIC_SCHEDULER: u8 = 15;

pub const STAGED_CAPACITY: usize = 512;

/// Thread-safe per-origin positive monotonic serial issuer.
#[derive(Debug)]
pub struct SerialIssuer {
    epoch: AtomicU64,
    next: [AtomicU32; ORIGINS],
}

impl Default for SerialIssuer {
    fn default() -> Self {
        Self::new()
    }
}

impl SerialIssuer {
    pub fn new() -> Self {
        Self {
            epoch: AtomicU64::new(1),
            next: core::array::from_fn(|_| AtomicU32::new(1)),
        }
    }

    pub fn epoch(&self) -> u64 {
        self.epoch.load(Ordering::Acquire)
    }

    pub fn next_serial(&self, origin: u8) -> u32 {
        let idx = usize::from(origin.min((ORIGINS - 1) as u8));
        loop {
            let cur = self.next[idx].load(Ordering::Relaxed);
            let next = if cur >= u32::MAX { 1 } else { cur + 1 };
            if self.next[idx]
                .compare_exchange_weak(cur, next, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                return cur.max(1);
            }
        }
    }

    pub fn advance_epoch_and_reset(&self) -> u64 {
        for s in &self.next {
            s.store(1, Ordering::Relaxed);
        }
        self.epoch.fetch_add(1, Ordering::AcqRel) + 1
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PhysicalMidiSource {
    /// Host OS ALSA/CoreMIDI/WASAPI/Android MidiManager owns hardware MIDI exclusively.
    Native,
    /// Browser WebMIDI owns the controller exclusively; native listener ignores hardware events.
    Browser,
}

/// Ensures physical MIDI hardware (e.g. CASIO controller) has a single canonical origin
/// instead of dual-path timestamp deduplication heuristics.
#[derive(Clone, Debug)]
pub struct CanonicalMidiArbiter {
    source: PhysicalMidiSource,
    next_note_id: u32,
}

impl Default for CanonicalMidiArbiter {
    fn default() -> Self {
        Self::new(PhysicalMidiSource::Native)
    }
}

impl CanonicalMidiArbiter {
    pub fn new(source: PhysicalMidiSource) -> Self {
        Self {
            source,
            next_note_id: 1,
        }
    }

    pub fn source(&self) -> PhysicalMidiSource {
        self.source
    }

    pub fn accepts(&self, candidate: PhysicalMidiSource) -> bool {
        self.source == candidate
    }

    pub fn switch_source(
        &mut self,
        next: PhysicalMidiSource,
        serials: &SerialIssuer,
        offset: u16,
    ) -> Option<Command> {
        if self.source == next {
            return None;
        }
        self.source = next;
        Some(Command {
            offset,
            origin: ORIGIN_HARDWARE_MIDI,
            serial: serials.next_serial(ORIGIN_HARDWARE_MIDI),
            action: Action::PanicOrigin,
        })
    }

    /// Stamps raw MIDI 1 / MPE event from the active canonical source onto `ORIGIN_HARDWARE_MIDI`.
    pub fn accept_raw_midi(
        &mut self,
        candidate: PhysicalMidiSource,
        status: u8,
        data1: u8,
        data2: u8,
        explicit_note_id: Option<u32>,
        offset: u16,
        serials: &SerialIssuer,
    ) -> Option<Command> {
        if !self.accepts(candidate) {
            return None;
        }
        let note_id = explicit_note_id.unwrap_or_else(|| {
            if matches!(status & 0xf0, 0x80 | 0x90) {
                u32::from(data1)
            } else {
                let id = self.next_note_id;
                self.next_note_id = self.next_note_id.wrapping_add(1).max(1);
                id
            }
        });
        Some(Command {
            offset,
            origin: ORIGIN_HARDWARE_MIDI,
            serial: serials.next_serial(ORIGIN_HARDWARE_MIDI),
            action: Action::Midi {
                status,
                data1,
                data2,
                note_id,
            },
        })
    }
}

/// Host-side MIDI Learn & CC mapping router preserving full lifecycle parity on `PreparedEngine`.
#[derive(Clone, Debug)]
pub struct CcLearnRouter {
    pub cc_map: [Option<u16>; 128],
    pub learning_target: Option<u16>,
}

impl Default for CcLearnRouter {
    fn default() -> Self {
        let mut cc_map = [None; 128];
        cc_map[1] = Some(ParamId::ModWheel as u16);
        cc_map[71] = Some(ParamId::FilterResonance as u16);
        cc_map[74] = Some(ParamId::FilterCutoff as u16);
        Self {
            cc_map,
            learning_target: None,
        }
    }
}

impl CcLearnRouter {
    pub fn begin_learn(&mut self, param_id: u16) -> bool {
        if usize::from(param_id) >= NUM_PARAMS || param_id == ParamId::OversamplingMode as u16 {
            return false;
        }
        self.learning_target = Some(param_id);
        true
    }

    pub fn cancel_learn(&mut self) {
        self.learning_target = None;
    }

    pub fn unlearn(&mut self, param_id: u16) -> bool {
        if usize::from(param_id) >= NUM_PARAMS {
            return false;
        }
        for slot in &mut self.cc_map {
            if *slot == Some(param_id) {
                *slot = None;
            }
        }
        if self.learning_target == Some(param_id) {
            self.learning_target = None;
        }
        true
    }

    pub fn remap(&mut self, cc: u8, param_id: u16) -> bool {
        if !AtomicParamBank::learnable_cc(usize::from(cc))
            || usize::from(param_id) >= NUM_PARAMS
            || param_id == ParamId::OversamplingMode as u16
        {
            return false;
        }
        self.unlearn(param_id);
        self.cc_map[usize::from(cc)] = Some(param_id);
        self.learning_target = None;
        true
    }

    /// Translates an incoming 3-byte MIDI message into either `Action::Parameter` (for mapped or
    /// learned general CCs) or `Action::Midi` (notes, pedals, pitch bend, channel pressure, MPE CC74).
    /// Returns `None` if the CC is unmapped and non-reserved.
    pub fn translate_midi(&mut self, status: u8, data1: u8, data2: u8, note_id: u32) -> Option<Action> {
        let kind = status & 0xf0;
        let ch = status & 0x0f;
        if kind != 0xb0 {
            return Some(Action::Midi {
                status,
                data1,
                data2,
                note_id,
            });
        }
        if matches!(data1, 64 | 66 | 120 | 123) || (ch > 0 && data1 == 74) {
            return Some(Action::Midi {
                status,
                data1,
                data2,
                note_id,
            });
        }
        if let Some(target) = self.learning_target {
            if self.remap(data1, target) {
                return Self::cc_to_param(target, data2);
            }
        }
        self.cc_map
            .get(usize::from(data1))
            .copied()
            .flatten()
            .and_then(|param_id| Self::cc_to_param(param_id, data2))
    }

    pub fn cc_to_param(param_id: u16, cc_val: u8) -> Option<Action> {
        let spec = PARAM_SPECS.get(usize::from(param_id))?;
        if spec.id == ParamId::OversamplingMode {
            return None;
        }
        let norm = f32::from(cc_val.min(127)) / 127.0;
        let raw = if spec.logarithmic {
            spec.min * (spec.max / spec.min).powf(norm)
        } else {
            spec.min + norm * (spec.max - spec.min)
        };
        let value = if spec.integer { raw.round() } else { raw }.clamp(spec.min, spec.max);
        Some(Action::Parameter {
            id: param_id,
            value,
        })
    }
}

/// Real-time musical clock / pattern / arpeggiator runner synced to `PreparedEngine`.
pub struct MusicTransport {
    pub scheduler: NoteScheduler,
    pub arp: Arpeggiator,
    pub arp_enabled: bool,
    pub bank: Box<PatternBank>,
    pub active_pattern: usize,
    pub enabled: bool,
    pub stop_pending_flush: bool,
    pub music_origin: u8,
}

impl MusicTransport {
    pub fn new(sample_rate: u32) -> Result<Self, MusicError> {
        Ok(Self {
            scheduler: NoteScheduler::new(sample_rate, 120_000, 0)?,
            arp: Arpeggiator::default(),
            arp_enabled: false,
            bank: Box::<PatternBank>::default(),
            active_pattern: 0,
            enabled: false,
            stop_pending_flush: false,
            music_origin: ORIGIN_MUSIC_SCHEDULER,
        })
    }

    pub fn start_from_zero(&mut self) -> Result<(), MusicError> {
        self.scheduler.seek(0)?;
        self.enabled = true;
        self.stop_pending_flush = false;
        Ok(())
    }

    pub fn start_grid(&mut self, step: u32, frame: u64) -> Result<(), MusicError> {
        self.scheduler.start_grid(step, frame)?;
        self.enabled = true;
        self.stop_pending_flush = false;
        Ok(())
    }

    pub fn seek(&mut self, frame: u64) -> Result<(), MusicError> {
        self.scheduler.seek(frame)?;
        self.enabled = true;
        self.stop_pending_flush = false;
        Ok(())
    }

    pub fn stop(&mut self) {
        if self.enabled {
            self.scheduler.stop();
            self.enabled = false;
            self.stop_pending_flush = true;
        }
    }

    pub fn needs_stop_flush(&self) -> bool {
        self.stop_pending_flush
    }

    pub fn should_render(&self) -> bool {
        self.enabled || self.stop_pending_flush
    }

    /// Executes a 1-frame non-empty block to deliver the pending `Trigger::Release`
    /// generated by `NoteScheduler::stop()` BEFORE suspending or discarding audio.
    pub fn flush_stop_before_suspend(
        &mut self,
        engine: &mut PreparedEngine,
    ) -> Result<(), PortError> {
        if !self.stop_pending_flush {
            return Ok(());
        }
        let pattern_idx = self.active_pattern.min(63);
        let pattern = &self.bank.0[pattern_idx];
        let arp_opt = self.arp_enabled.then_some(&self.arp);
        let mut music_buf = [MusicEvent::default(); MAX_EVENTS];
        let mut status_buf = [Status::Invalid; MAX_EVENTS];
        let mut trial = self.scheduler;
        let n = trial
            .block(1, pattern, arp_opt, &mut music_buf)
            .map_err(|_| PortError::Capacity)?;
        let mut l = [0.0f32; 1];
        let mut r = [0.0f32; 1];
        engine.process_music(
            &mut l,
            &mut r,
            &[],
            &mut [],
            self.music_origin,
            &music_buf[..n],
            &mut status_buf[..n],
        )?;
        self.scheduler = trial;
        self.stop_pending_flush = false;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BlockSummary {
    pub frames_rendered: usize,
    pub sub_blocks: usize,
    pub carried_in: usize,
    pub carried_out: usize,
    pub shifted_overflow_events: usize,
    pub host_acks: usize,
    pub music_results: usize,
}

/// Zero-allocation audio-thread block coordinator for ordered EnginePort v1 commands,
/// right-endpoint carry (`offset == frames`), `max_frames` / `256`-event splitting,
/// and transactional `process_music` segment scheduling.
pub struct BlockScheduler {
    carried: [Command; MAX_EVENTS],
    carried_len: usize,
    dropped_carry_overflow: u64,
    scratch: [(usize, Command); STAGED_CAPACITY],
    sub_cmds: [Command; MAX_EVENTS],
    ack_scratch: [Ack; MAX_EVENTS],
    music_scratch: [MusicEvent; MAX_EVENTS],
    music_status_scratch: [Status; MAX_EVENTS],
}

impl Default for BlockScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl BlockScheduler {
    pub fn new() -> Self {
        let zero_cmd = Command {
            offset: 0,
            origin: 0,
            serial: 0,
            action: Action::PanicOrigin,
        };
        Self {
            carried: [zero_cmd; MAX_EVENTS],
            carried_len: 0,
            dropped_carry_overflow: 0,
            scratch: [(0, zero_cmd); STAGED_CAPACITY],
            sub_cmds: [zero_cmd; MAX_EVENTS],
            ack_scratch: [Ack::default(); MAX_EVENTS],
            music_scratch: [MusicEvent::default(); MAX_EVENTS],
            music_status_scratch: [Status::Invalid; MAX_EVENTS],
        }
    }

    pub fn carried_len(&self) -> usize {
        self.carried_len
    }

    pub fn dropped_carry_overflow(&self) -> u64 {
        self.dropped_carry_overflow
    }

    pub fn clear_carried(&mut self) {
        self.carried_len = 0;
    }

    /// Processes a host audio block `(left, right)` with incoming commands (with offsets in `0..=frames`).
    /// - Offsets strictly `> frames` return `Err(PortError::Offset)` atomically before mutating state or buffers.
    /// - Commands at right endpoint `offset == frames` are carried to `offset = 0` of the next non-empty block.
    /// - Equal-offset commands preserve causal arrival order (`carried` precedes `incoming`, `incoming` stays ordered).
    /// - Any `PortError` clears `left` and `right` to silence so partial PCM is never exposed.
    #[allow(clippy::too_many_arguments)]
    pub fn process_block<F, M>(
        &mut self,
        engine: &mut PreparedEngine,
        left: &mut [f32],
        right: &mut [f32],
        incoming: &[Command],
        mut ack_sink: F,
        mut music: Option<&mut MusicTransport>,
        mut music_status_sink: M,
    ) -> Result<BlockSummary, PortError>
    where
        F: FnMut(Ack),
        M: FnMut(Status),
    {
        if left.len() != right.len() {
            left.fill(0.0);
            right.fill(0.0);
            return Err(PortError::Block);
        }
        let total_frames = left.len();

        for c in incoming {
            if usize::from(c.offset) > total_frames {
                left.fill(0.0);
                right.fill(0.0);
                return Err(PortError::Offset);
            }
            if usize::from(c.origin) >= ORIGINS || c.serial == 0 {
                left.fill(0.0);
                right.fill(0.0);
                return Err(PortError::Origin);
            }
        }

        if total_frames == 0 {
            for &c in incoming {
                let mut moved = c;
                moved.offset = 0;
                if self.carried_len < MAX_EVENTS {
                    self.carried[self.carried_len] = moved;
                    self.carried_len += 1;
                } else {
                    self.dropped_carry_overflow += 1;
                }
            }
            return Ok(BlockSummary {
                frames_rendered: 0,
                sub_blocks: 0,
                carried_in: self.carried_len,
                carried_out: self.carried_len,
                ..Default::default()
            });
        }

        let carried_in = self.carried_len;
        let mut staged_len = 0usize;
        let mut next_carried = [Command {
            offset: 0,
            origin: 0,
            serial: 0,
            action: Action::PanicOrigin,
        }; MAX_EVENTS];
        let mut next_carried_len = 0usize;

        for i in 0..self.carried_len {
            let mut c = self.carried[i];
            c.offset = 0;
            if staged_len < STAGED_CAPACITY {
                self.scratch[staged_len] = (staged_len, c);
                staged_len += 1;
            } else {
                self.dropped_carry_overflow += 1;
            }
        }
        for &c in incoming {
            if usize::from(c.offset) == total_frames {
                let mut carried_cmd = c;
                carried_cmd.offset = 0;
                if next_carried_len < MAX_EVENTS {
                    next_carried[next_carried_len] = carried_cmd;
                    next_carried_len += 1;
                } else {
                    self.dropped_carry_overflow += 1;
                }
            } else if staged_len < STAGED_CAPACITY {
                self.scratch[staged_len] = (staged_len, c);
                staged_len += 1;
            } else if next_carried_len < MAX_EVENTS {
                let mut overflow_cmd = c;
                overflow_cmd.offset = 0;
                next_carried[next_carried_len] = overflow_cmd;
                next_carried_len += 1;
                self.dropped_carry_overflow += 1;
            } else {
                self.dropped_carry_overflow += 1;
            }
        }

        // Zero-allocation stable insertion sort by `(offset, arrival_seq)`.
        for i in 1..staged_len {
            let item = self.scratch[i];
            let key = (item.1.offset, item.0);
            let mut j = i;
            while j > 0 {
                let prev = self.scratch[j - 1];
                if (prev.1.offset, prev.0) <= key {
                    break;
                }
                self.scratch[j] = prev;
                j -= 1;
            }
            self.scratch[j] = item;
        }

        let max_frames = usize::from(engine.configuration().max_frames).max(1);
        let mut pos = 0usize;
        let mut cmd_idx = 0usize;
        let mut summary = BlockSummary {
            frames_rendered: total_frames,
            carried_in,
            ..Default::default()
        };

        while pos < total_frames {
            let max_end = (pos + max_frames).min(total_frames);
            let mut limit_idx = cmd_idx;
            while limit_idx < staged_len && usize::from(self.scratch[limit_idx].1.offset) < max_end {
                limit_idx += 1;
            }

            let mut sub_end = max_end;
            let host_avail = limit_idx - cmd_idx;
            if host_avail > MAX_EVENTS {
                let spill_offset = usize::from(self.scratch[cmd_idx + MAX_EVENTS].1.offset);
                if spill_offset > pos {
                    sub_end = spill_offset;
                } else {
                    // >256 events on exact sample `pos`: process first 256 and advance overflow.
                    sub_end = pos + 1;
                    let target_offset = if pos + 1 < total_frames {
                        (pos + 1) as u16
                    } else {
                        u16::MAX
                    };
                    for idx in (cmd_idx + MAX_EVENTS)..limit_idx {
                        if usize::from(self.scratch[idx].1.offset) == pos {
                            if target_offset != u16::MAX {
                                self.scratch[idx].1.offset = target_offset;
                                summary.shifted_overflow_events += 1;
                            } else if next_carried_len < MAX_EVENTS {
                                let mut mc = self.scratch[idx].1;
                                mc.offset = 0;
                                next_carried[next_carried_len] = mc;
                                next_carried_len += 1;
                                self.scratch[idx].1.offset = u16::MAX;
                                summary.shifted_overflow_events += 1;
                            }
                        } else {
                            break;
                        }
                    }
                    // Strip sentinel MAX-offset moved items at tail safely during loop progression.
                }
            }

            let render_music = music.as_deref().is_some_and(MusicTransport::should_render);
            if render_music {
                let transport = music.as_deref_mut().unwrap();
                let pattern_idx = transport.active_pattern.min(63);
                let pattern = &transport.bank.0[pattern_idx];
                let arp_opt = transport.arp_enabled.then_some(&transport.arp);

                let mut try_end = sub_end;
                let (m_count, host_take, chosen_trial) = loop {
                    let frames_u16 = (try_end - pos) as u16;
                    let mut avail_host = 0usize;
                    while cmd_idx + avail_host < staged_len
                        && usize::from(self.scratch[cmd_idx + avail_host].1.offset) < try_end
                    {
                        avail_host += 1;
                    }
                    let mut trial = transport.scheduler;
                    match trial.block(frames_u16, pattern, arp_opt, &mut self.music_scratch) {
                        Ok(cnt) => {
                            if avail_host + cnt <= MAX_EVENTS {
                                break (cnt, avail_host, trial);
                            }
                            if frames_u16 > 1 {
                                try_end = pos + usize::from((frames_u16 / 2).max(1));
                                continue;
                            }
                            let host_take = avail_host.min(MAX_EVENTS.saturating_sub(cnt));
                            for idx in (cmd_idx + host_take)..(cmd_idx + avail_host) {
                                if pos + 1 < total_frames {
                                    self.scratch[idx].1.offset = (pos + 1) as u16;
                                } else if next_carried_len < MAX_EVENTS {
                                    let mut mc = self.scratch[idx].1;
                                    mc.offset = 0;
                                    next_carried[next_carried_len] = mc;
                                    next_carried_len += 1;
                                    self.scratch[idx].1.offset = u16::MAX;
                                } else {
                                    self.scratch[idx].1.offset = u16::MAX;
                                    self.dropped_carry_overflow += 1;
                                }
                                summary.shifted_overflow_events += 1;
                            }
                            break (cnt, host_take, trial);
                        }
                        Err(MusicError::Capacity) if frames_u16 > 1 => {
                            try_end = pos + usize::from((frames_u16 / 2).max(1));
                        }
                        Err(_) => {
                            left.fill(0.0);
                            right.fill(0.0);
                            return Err(PortError::Capacity);
                        }
                    }
                };

                for i in 0..host_take {
                    let mut c = self.scratch[cmd_idx + i].1;
                    c.offset -= pos as u16;
                    self.sub_cmds[i] = c;
                }
                if let Err(err) = engine.process_music(
                    &mut left[pos..try_end],
                    &mut right[pos..try_end],
                    &self.sub_cmds[..host_take],
                    &mut self.ack_scratch[..host_take],
                    transport.music_origin,
                    &self.music_scratch[..m_count],
                    &mut self.music_status_scratch[..m_count],
                ) {
                    left.fill(0.0);
                    right.fill(0.0);
                    return Err(err);
                }

                // Commit copied scheduler ONLY after accepted processing.
                transport.scheduler = chosen_trial;
                if transport.stop_pending_flush {
                    transport.stop_pending_flush = false;
                }
                for ack in &self.ack_scratch[..host_take] {
                    ack_sink(*ack);
                    summary.host_acks += 1;
                }
                for &st in &self.music_status_scratch[..m_count] {
                    music_status_sink(st);
                    summary.music_results += 1;
                }

                cmd_idx += host_take;
                Self::resort_tail(&mut self.scratch, cmd_idx, &mut staged_len);
                pos = try_end;
                summary.sub_blocks += 1;
            } else {
                let mut host_take = 0usize;
                while host_take < MAX_EVENTS
                    && cmd_idx + host_take < staged_len
                    && usize::from(self.scratch[cmd_idx + host_take].1.offset) < sub_end
                {
                    host_take += 1;
                }
                for i in 0..host_take {
                    let mut c = self.scratch[cmd_idx + i].1;
                    c.offset -= pos as u16;
                    self.sub_cmds[i] = c;
                }
                if let Err(err) = engine.process(
                    &mut left[pos..sub_end],
                    &mut right[pos..sub_end],
                    &self.sub_cmds[..host_take],
                    &mut self.ack_scratch[..host_take],
                ) {
                    left.fill(0.0);
                    right.fill(0.0);
                    return Err(err);
                }
                for ack in &self.ack_scratch[..host_take] {
                    ack_sink(*ack);
                    summary.host_acks += 1;
                }
                cmd_idx += host_take;
                Self::resort_tail(&mut self.scratch, cmd_idx, &mut staged_len);
                pos = sub_end;
                summary.sub_blocks += 1;
            }
        }

        // Preserve any next_carried events safely.
        self.carried[..next_carried_len].copy_from_slice(&next_carried[..next_carried_len]);
        self.carried_len = next_carried_len;
        summary.carried_out = next_carried_len;
        Ok(summary)
    }

    fn resort_tail(
        scratch: &mut [(usize, Command); STAGED_CAPACITY],
        start: usize,
        staged_len: &mut usize,
    ) {
        if start >= *staged_len {
            return;
        }
        for i in (start + 1)..*staged_len {
            let item = scratch[i];
            let key = (item.1.offset, item.0);
            let mut j = i;
            while j > start {
                let prev = scratch[j - 1];
                if (prev.1.offset, prev.0) <= key {
                    break;
                }
                scratch[j] = prev;
                j -= 1;
            }
            scratch[j] = item;
        }
        while *staged_len > start && scratch[*staged_len - 1].1.offset == u16::MAX {
            *staged_len -= 1;
        }
    }
}

/// Helper flag indicating whether any worker replacement requested an urgent quiet stop.
#[derive(Debug, Default)]
pub struct PendingSwapFlag {
    pending: AtomicBool,
}

impl PendingSwapFlag {
    pub fn mark_pending(&self) {
        self.pending.store(true, Ordering::Release);
    }

    pub fn take_pending(&self) -> bool {
        self.pending.swap(false, Ordering::AcqRel)
    }
}
