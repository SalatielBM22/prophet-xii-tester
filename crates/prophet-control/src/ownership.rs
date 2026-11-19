use super::ProphetXiiPlugin;
use crate::port::{Status, ORIGINS};

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq)]
pub struct Note {
    pub origin: u8,
    pub channel: u8,
    pub key: u8,
    pub note_id: u32,
    pub pressed: bool,
    pub sostenuto: bool,
    generation: u64,
    bend: i16,
    velocity: u8,
    pressure: u8,
    timbre: Option<u8>,
}
#[derive(Default, Copy, Clone)]
struct Pedals {
    damper: bool,
    sostenuto: bool,
}
pub(crate) struct Ownership {
    notes: [Option<Note>; 128],
    pedals: [[Pedals; 16]; ORIGINS],
}
impl Default for Ownership {
    fn default() -> Self {
        Self {
            notes: [None; 128],
            pedals: Default::default(),
        }
    }
}
impl ProphetXiiPlugin {
    pub fn held_notes(&self) -> impl Iterator<Item = &Note> {
        self.ownership.notes.iter().flatten()
    }

    // The allocator can grow a stack or return to an older sustained key.
    // Expressions belong to its MIDI note identity, independent of voice index.
    pub(crate) fn refresh_stack(&mut self) {
        if !self.kernel.allocator.unison_enabled {
            return;
        }
        let first = &self.kernel.allocator.voices[0];
        if !first.gate {
            return;
        }
        let key = first.midi_note;
        let selected = self
            .held_notes()
            .filter(|n| n.key == key)
            .max_by_key(|n| n.generation)
            .copied();
        if let Some(note) = selected {
            for v in &mut self.kernel.allocator.voices {
                if v.gate && v.midi_note == key {
                    note.apply_expression(v);
                    v.note_age = note.generation;
                }
            }
        }
    }
    /// The legacy allocator tracks keys. Restore distinct MIDI note identities
    /// (including overlapping equal keys) when a mono stack returns to poly.
    pub(crate) fn restore_poly_owners(&mut self) {
        use prophet_dsp::voice::NUM_VOICES;
        let mut held = self.ownership.notes;
        held.sort_unstable_by_key(|n| core::cmp::Reverse(n.map(|n| n.generation)));
        let a = &mut self.kernel.allocator;
        let mut used = [false; NUM_VOICES];
        let mut missing = [None; NUM_VOICES];
        for (j, n) in held[..NUM_VOICES].iter().flatten().enumerate() {
            let index = a
                .voices
                .iter()
                .enumerate()
                .filter(|(i, v)| !used[*i] && v.gate && v.midi_note == n.key)
                .max_by_key(|(_, v)| v.note_age == n.generation)
                .map(|(i, _)| i);
            if let Some(i) = index {
                used[i] = true;
                let v = &mut a.voices[i];
                v.note_age = n.generation;
                v.velocity = f32::from(n.velocity) / 127.;
                n.apply_expression(v);
            } else {
                missing[j] = Some(*n);
            }
        }
        for (v, used) in a.voices.iter_mut().zip(used) {
            if !used && v.gate {
                v.trigger_note_off();
            }
        }
        for n in missing.into_iter().flatten() {
            a.note_on_with_reuse(
                n.key,
                f32::from(n.velocity) / 127.,
                &self.kernel.voice_params,
                false,
            );
            if let Some(v) = a
                .voices
                .iter_mut()
                .find(|v| v.gate && v.note_age == a.event_counter)
            {
                v.note_age = n.generation;
                n.apply_expression(v);
            }
        }
    }
    fn drop_note(&mut self, slot: usize) {
        let Some(note) = self.ownership.notes[slot].take() else {
            return;
        };
        let shared = self.held_notes().any(|n| n.key == note.key);
        let a = &mut self.kernel.allocator;
        if !shared {
            a.note_off(note.key, &self.kernel.voice_params);
        } else if !a.unison_enabled {
            for v in &mut a.voices {
                if v.gate && v.note_age == note.generation && v.midi_note == note.key {
                    v.trigger_note_off();
                }
            }
        }
    }
    pub(crate) fn release_source(&mut self, origin: u8, channel: Option<u8>) {
        for i in 0..128 {
            if self.ownership.notes[i]
                .is_some_and(|n| n.origin == origin && channel.is_none_or(|ch| n.channel == ch))
            {
                self.drop_note(i);
            }
        }
        for (ch, pedals) in self.ownership.pedals[origin as usize]
            .iter_mut()
            .enumerate()
        {
            if channel.is_none_or(|c| usize::from(c) == ch) {
                *pedals = Pedals::default();
            }
        }
        // This origin's damper may have been the only one holding notes that
        // other origins released on the same channel.
        for ch in 0..16u8 {
            if channel.is_none_or(|c| c == ch) {
                self.release_unheld(ch);
            }
        }
        self.refresh_stack();
    }
    /// The damper (CC64) acts as one physical pedal per MIDI channel: a pedal
    /// held by any origin sustains keys released on that channel by every
    /// origin, so a hardware pedal holds notes played on the local UI keyboard.
    /// Sostenuto (CC66) stays per origin because it captures that origin's keys.
    fn damper_down(&self, ch: u8) -> bool {
        self.ownership
            .pedals
            .iter()
            .any(|origin| origin[usize::from(ch)].damper)
    }
    fn release_unheld(&mut self, ch: u8) {
        if self.damper_down(ch) {
            return;
        }
        for i in 0..128 {
            if self.ownership.notes[i]
                .is_some_and(|n| n.channel == ch && !n.pressed && !n.sostenuto)
            {
                self.drop_note(i);
            }
        }
    }
    pub(crate) fn midi_owned(&mut self, origin: u8, status: u8, d1: u8, d2: u8, id: u32) -> Status {
        if origin as usize >= ORIGINS || d1 > 127 || d2 > 127 || !(0x80..=0xef).contains(&status) {
            return Status::Invalid;
        }
        let ch = status & 15;
        let find = |n: &Note| (n.origin, n.channel, n.key, n.note_id) == (origin, ch, d1, id);
        match status & 0xf0 {
            0x90 if d2 > 0 => {
                let existing = self
                    .ownership
                    .notes
                    .iter()
                    .position(|n| n.as_ref().is_some_and(find));
                let Some(i) =
                    existing.or_else(|| self.ownership.notes.iter().position(Option::is_none))
                else {
                    return Status::Capacity;
                };
                let overlap = self
                    .ownership
                    .notes
                    .iter()
                    .enumerate()
                    .any(|(j, n)| j != i && n.is_some_and(|n| n.key == d1));
                if existing.is_some() && overlap {
                    self.drop_note(i);
                }
                self.kernel.allocator.note_on_with_reuse(
                    d1,
                    f32::from(d2) / 127.0,
                    &self.kernel.voice_params,
                    !overlap,
                );
                self.ownership.notes[i] = Some(Note {
                    origin,
                    channel: ch,
                    key: d1,
                    note_id: id,
                    pressed: true,
                    velocity: d2,
                    sostenuto: false,
                    generation: self.kernel.allocator.event_counter,
                    ..Default::default()
                });
            }
            0x80 | 0x90 => {
                if let Some(i) = self
                    .ownership
                    .notes
                    .iter()
                    .position(|n| n.as_ref().is_some_and(find))
                {
                    let damper = self.damper_down(ch);
                    let n = self.ownership.notes[i].as_mut().unwrap();
                    n.pressed = false;
                    if !damper && !n.sostenuto {
                        self.drop_note(i);
                    }
                }
            }
            0xb0 if d1 == 64 || d1 == 66 => {
                let pedals = &mut self.ownership.pedals[origin as usize][ch as usize];
                let down = d2 >= 64;
                let capture = d1 == 66 && down && !pedals.sostenuto;
                if d1 == 64 {
                    pedals.damper = down
                } else {
                    pedals.sostenuto = down
                }
                for n in self.ownership.notes.iter_mut().flatten() {
                    if n.origin != origin || n.channel != ch {
                        continue;
                    }
                    if capture {
                        n.sostenuto = n.pressed
                    }
                    if d1 == 66 && !down {
                        n.sostenuto = false
                    }
                }
                self.release_unheld(ch);
            }
            0xb0 if d1 == 120 || d1 == 123 => self.release_source(origin, Some(ch)),
            0xb0 | 0xd0 | 0xe0 => {
                if ch > 0 && (status & 0xf0 != 0xb0 || d1 == 74) {
                    let n = self
                        .ownership
                        .notes
                        .iter_mut()
                        .flatten()
                        .filter(|n| n.origin == origin && n.channel == ch)
                        .max_by_key(|n| n.generation);
                    if let Some(n) = n {
                        match status & 0xf0 {
                            0xb0 => n.timbre = Some(d2),
                            0xd0 => n.pressure = d1,
                            _ => n.bend = i16::from(d2) * 128 + i16::from(d1) - 8192,
                        }
                        for v in &mut self.kernel.allocator.voices {
                            if v.gate && v.note_age == n.generation && v.midi_note == n.key {
                                n.apply_expression(v);
                            }
                        }
                    }
                } else {
                    self.global_controller(status, d1, d2);
                }
            }
            _ => return Status::Unsupported,
        }
        self.refresh_stack();
        Status::Applied
    }
}

impl Note {
    fn apply_expression(&self, v: &mut prophet_dsp::voice::ProphetVoice) {
        v.mpe_timbre = self.timbre.map_or(0., |t| f32::from(t) / 127. - 0.5);
        v.mpe_pressure = f32::from(self.pressure) / 127.;
        v.mpe_pitch_bend_semis = f32::from(self.bend) * (48. / 8192.);
    }
}
