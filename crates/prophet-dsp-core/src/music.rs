//! Sixteenth-note transport. All boundaries are derived from the absolute
//! step number, avoiding cumulative rounding drift at fractional tempos.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Rest,
    Note,
    Tie,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Step {
    pub kind: Kind,
    pub key: u8,
    pub velocity: u8,
    pub accent: bool,
    pub glide: bool,
    pub gate: u8,
    pub ratchet: u8,
}
impl Default for Step {
    fn default() -> Self {
        Self {
            kind: Kind::Rest,
            key: 60,
            velocity: 96,
            accent: false,
            glide: false,
            gate: 75,
            ratchet: 1,
        }
    }
}
impl Step {
    pub fn valid(self) -> bool {
        self.key < 128
            && (1..=127).contains(&self.velocity)
            && (1..=100).contains(&self.gate)
            && (1..=4).contains(&self.ratchet)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pattern {
    pub steps: [Step; 32],
    pub len: u8,
}
impl Default for Pattern {
    fn default() -> Self {
        Self {
            steps: [Step::default(); 32],
            len: 32,
        }
    }
}
impl Pattern {
    pub fn valid(&self) -> bool {
        (1..=32).contains(&self.len) && self.steps.iter().all(|s| s.valid())
    }
    pub fn page(&self, page: usize) -> Option<&[Step]> {
        self.steps.chunks_exact(8).nth(page)
    }
}
#[derive(Clone)]
pub struct PatternBank(pub [Pattern; 64]);
impl Default for PatternBank {
    fn default() -> Self {
        Self([Pattern::default(); 64])
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    Up,
    Down,
    UpDown,
    AsPlayed,
    Random,
    Chord,
}
/// Insertion order is retained separately from pitch order. A chord is bounded
/// by voice count. When latched, the next new chord replaces the released one.
#[derive(Clone, Debug)]
pub struct Arpeggiator {
    keys: [u8; 12],
    pressed: [bool; 12],
    n: usize,
    pub order: Order,
    pub latch: bool,
    pub seed: u32,
    octaves: u8,
}
impl Default for Arpeggiator {
    fn default() -> Self {
        Self {
            keys: [0; 12],
            pressed: [false; 12],
            n: 0,
            order: Order::Up,
            latch: false,
            seed: 1,
            octaves: 1,
        }
    }
}
impl Arpeggiator {
    pub fn set_octaves(&mut self, n: u8) -> bool {
        if !(1..=4).contains(&n) {
            return false;
        }
        self.octaves = n;
        true
    }
    pub fn key(&mut self, key: u8, on: bool) -> bool {
        if key > 127 {
            return false;
        }
        if on && self.latch && !self.pressed[..self.n].contains(&true) {
            self.n = 0;
        }
        if let Some(i) = self.keys[..self.n].iter().position(|k| *k == key) {
            self.pressed[i] = on;
            if !on && !self.latch {
                self.keys.copy_within(i + 1..self.n, i);
                self.pressed.copy_within(i + 1..self.n, i);
                self.n -= 1;
            }
        } else if on {
            if self.n == 12 {
                return false;
            }
            self.keys[self.n] = key;
            self.pressed[self.n] = true;
            self.n += 1;
        }
        true
    }
    fn notes(&self, tick: u64, out: &mut [u8; 12]) -> usize {
        let mut keys = self.keys;
        let n = self.n;
        if n == 0 {
            return 0;
        }
        if self.order != Order::AsPlayed {
            keys[..n].sort_unstable();
        }
        let mut expanded = [0u8; 48];
        let mut count = 0;
        for octave in 0..self.octaves {
            for &key in &keys[..n] {
                let key = key + 12 * octave;
                if key < 128 {
                    expanded[count] = key;
                    count += 1;
                }
            }
        }
        if self.order == Order::Chord {
            let oct = (tick % u64::from(self.octaves)) as u8;
            let mut used = 0;
            for &k in &keys[..n] {
                if k + 12 * oct < 128 {
                    out[used] = k + 12 * oct;
                    used += 1;
                }
            }
            return used;
        }
        if matches!(self.order, Order::Up | Order::Down | Order::UpDown) {
            expanded[..count].sort_unstable();
        }
        let i = (tick % count as u64) as usize;
        let index = match self.order {
            Order::Down => count - 1 - i,
            Order::UpDown if count > 1 => {
                let period = 2 * (count - 1);
                let j = (tick % period as u64) as usize;
                j.min(period - j)
            }
            Order::Random => {
                let mut v = (tick as u32 ^ self.seed).wrapping_mul(0x9e3779b9);
                v ^= v >> 16;
                v = v.wrapping_mul(0x85ebca6b);
                (v as usize) % count
            }
            _ => i,
        };
        out[0] = expanded[index];
        1
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Trigger {
    #[default]
    Release,
    Note {
        key: u8,
        velocity: u8,
        glide: bool,
    },
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MusicEvent {
    pub offset: u16,
    pub trigger: Trigger,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MusicError {
    Configuration,
    Capacity,
    Position,
}
#[derive(Clone, Copy, Debug)]
pub struct NoteScheduler {
    rate: u32,
    tempo_milli: u32,
    swing: u8,
    frame: u64,
    step: u64,
    pulse: u8,
    next: u64,
    release: Option<u64>,
    playing: bool,
    flush: bool,
}
impl NoteScheduler {
    pub fn new(rate: u32, tempo_milli: u32, swing: u8) -> Result<Self, MusicError> {
        if !(8000..=192000).contains(&rate)
            || !(20000..=400000).contains(&tempo_milli)
            || swing > 75
        {
            return Err(MusicError::Configuration);
        }
        Ok(Self {
            rate,
            tempo_milli,
            swing,
            frame: 0,
            step: 0,
            pulse: 0,
            next: 0,
            release: None,
            playing: false,
            flush: false,
        })
    }
    fn boundary(&self, step: u64) -> u64 {
        let ticks = 100 * u128::from(step) + u128::from(step % 2) * u128::from(self.swing);
        (ticks * u128::from(self.rate) * 150 / u128::from(self.tempo_milli)) as u64
    }
    /// Seek/loop at a host split point. Mid-step seeks resume on the next grid onset.
    pub fn seek(&mut self, frame: u64) -> Result<(), MusicError> {
        if frame > 1 << 52 {
            return Err(MusicError::Position);
        }
        let straight = (u128::from(frame) * u128::from(self.tempo_milli)
            / (15000 * u128::from(self.rate))) as u64;
        self.step = straight.saturating_sub(1);
        while self.boundary(self.step) < frame {
            self.step += 1;
        }
        self.playing = true;
        self.frame = frame;
        self.next = self.boundary(self.step);
        self.pulse = 0;
        self.release = None;
        self.flush = true;
        Ok(())
    }
    pub fn stop(&mut self) {
        self.playing = false;
        self.release = None;
        self.flush = true;
    }
    /// On failure the transport does not advance. Only consume output on Ok.
    pub fn block(
        &mut self,
        frames: u16,
        p: &Pattern,
        arp: Option<&Arpeggiator>,
        out: &mut [MusicEvent],
    ) -> Result<usize, MusicError> {
        if frames == 0 || frames > 1024 || !p.valid() {
            return Err(MusicError::Configuration);
        }
        let mut t = *self;
        let end = t.frame + u64::from(frames);
        if end > 1 << 52 {
            return Err(MusicError::Position);
        }
        let mut used = 0;
        let mut emit = |frame: u64, trigger| {
            *out.get_mut(used).ok_or(MusicError::Capacity)? = MusicEvent {
                offset: (frame - self.frame) as u16,
                trigger,
            };
            used += 1;
            Ok(())
        };
        if t.flush {
            emit(t.frame, Trigger::Release)?;
            t.flush = false;
        }
        while t.playing {
            let at = t.next.min(t.release.unwrap_or(u64::MAX));
            if at >= end {
                break;
            }
            if t.release == Some(at) {
                emit(at, Trigger::Release)?;
                t.release = None;
                continue;
            }
            let idx = (t.step % u64::from(p.len)) as usize;
            let step = p.steps[idx];
            let start = t.boundary(t.step);
            let limit = t.boundary(t.step + 1);
            let pulses = if step.kind == Kind::Note {
                step.ratchet
            } else {
                1
            };
            // Pattern edits take effect at a split point. A smaller ratchet
            // count retires any remaining substeps of the previous setting.
            if t.pulse >= pulses {
                t.step += 1;
                t.pulse = 0;
                t.next = limit;
                continue;
            }
            let following = start + (limit - start) * u64::from(t.pulse + 1) / u64::from(pulses);
            let follows_tie =
                p.steps[(idx + 1) % usize::from(p.len)].kind == Kind::Tie && t.pulse + 1 == pulses;
            if step.kind == Kind::Note {
                emit(at, Trigger::Release)?;
                let mut keys = [step.key; 12];
                let n = arp.map_or(1, |a| a.notes(t.step, &mut keys));
                for &key in &keys[..n] {
                    emit(
                        at,
                        Trigger::Note {
                            key,
                            velocity: step
                                .velocity
                                .saturating_add(if step.accent { 16 } else { 0 })
                                .min(127),
                            glide: step.glide,
                        },
                    )?;
                }
                if !follows_tie {
                    t.release = Some(
                        at + ((following.saturating_sub(at)) * u64::from(step.gate) / 100).max(1),
                    );
                }
            } else if step.kind == Kind::Rest {
                emit(at, Trigger::Release)?;
            } else if !follows_tie {
                t.release = Some(following);
            }
            t.pulse += 1;
            if t.pulse >= pulses {
                t.step += 1;
                t.pulse = 0;
            }
            t.next = following.max(at + 1);
        }
        if t.playing {
            t.frame = end;
        }
        *self = t;
        Ok(used)
    }
}
