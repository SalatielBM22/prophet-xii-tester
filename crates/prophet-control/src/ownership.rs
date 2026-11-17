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
                    sostenuto: false,
                    generation: self.kernel.allocator.event_counter,
                });
            }
            0x80 | 0x90 => {
                if let Some(i) = self
                    .ownership
                    .notes
                    .iter()
                    .position(|n| n.as_ref().is_some_and(find))
                {
                    let n = self.ownership.notes[i].as_mut().unwrap();
                    n.pressed = false;
                    if !self.ownership.pedals[origin as usize][ch as usize].damper && !n.sostenuto {
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
                let damper = pedals.damper;
                for i in 0..128 {
                    if let Some(n) = &mut self.ownership.notes[i] {
                        if n.origin != origin || n.channel != ch {
                            continue;
                        }
                        if capture {
                            n.sostenuto = n.pressed
                        }
                        if d1 == 66 && !down {
                            n.sostenuto = false
                        }
                        if !n.pressed && !n.sostenuto && !damper {
                            self.drop_note(i)
                        }
                    }
                }
            }
            0xb0 if d1 == 120 || d1 == 123 => self.release_source(origin, Some(ch)),
            0xb0 | 0xd0 | 0xe0 => {
                if ch > 0 && (status & 0xf0 != 0xb0 || d1 == 74) {
                    let n = self
                        .held_notes()
                        .filter(|n| n.origin == origin && n.channel == ch)
                        .max_by_key(|n| n.generation)
                        .copied();
                    if let Some(n) = n {
                        for v in &mut self.kernel.allocator.voices {
                            if v.gate
                                && v.midi_note == n.key
                                && (v.note_age == n.generation
                                    || self.kernel.allocator.unison_enabled)
                            {
                                match status & 0xf0 {
                                    0xb0 => v.mpe_timbre = f32::from(d2) / 127.0 - 0.5,
                                    0xd0 => v.mpe_pressure = f32::from(d1) / 127.0,
                                    _ => {
                                        v.mpe_pitch_bend_semis =
                                            (f32::from(u16::from(d2) * 128 + u16::from(d1))
                                                - 8192.0)
                                                / 8192.0
                                                * 48.0
                                    }
                                }
                            }
                        }
                    }
                } else {
                    self.global_controller(status, d1, d2);
                }
            }
            _ => return Status::Unsupported,
        }
        Status::Applied
    }
}
