//! Worker-only native preset codec. Payload CRC is over compact, typed JSON
//! in declaration order. Unknown sound fields or revisions reject the recall.
use prophet_control::{
    music::{Kind, Pattern, PatternBank, Step},
    state::SoundState,
};
use prophet_dsp::{
    fx::reverb::Algorithm,
    vector::{Patch, Point, Route, Stage, Topology},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Size,
    Json,
    Version,
    Checksum,
    Parameters,
    Algorithm,
    Pattern,
}
pub struct Preset {
    pub name: String,
    pub category: String,
    pub sound: SoundState,
    pub bank: Option<Box<PatternBank>>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Native {
    format: String,
    version: u32,
    payload: Payload,
    crc32: u32,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    name: String,
    category: String,
    values: Vec<f32>,
    glide: bool,
    drive_makeup: bool,
    reverb: [u8; 2],
    enabled: bool,
    kill_tail: bool,
    oscillator: [u8; 2],
    routes: [u8; 2],
    ratios: [f32; 2],
    xy: [f32; 2],
    stages: [([f32; 2], u16); 5],
    loop_held: bool,
    patterns: Option<Vec<(u8, [[u8; 7]; 32])>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Legacy {
    format: String,
    version: u32,
    name: String,
    category: String,
    values: Vec<f32>,
}
#[derive(Deserialize)]
struct Header {
    format: String,
    version: u32,
}
fn crc(bytes: &[u8]) -> u32 {
    let mut s = !0u32;
    for b in bytes {
        s ^= u32::from(*b);
        for _ in 0..8 {
            s = (s >> 1) ^ ((0u32.wrapping_sub(s & 1)) & 0xedb88320);
        }
    }
    !s
}
fn xy(p: Point) -> [f32; 2] {
    [p.x, p.y]
}
fn point([x, y]: [f32; 2]) -> Point {
    Point { x, y }
}
const MAX_BYTES: usize = 512 * 1024;
impl Preset {
    pub fn encode(&self) -> Result<Vec<u8>, Error> {
        self.sound.validate().map_err(|_| Error::Parameters)?;
        let s = &self.sound;
        let p = s.patch;
        let patterns = if let Some(bank) = &self.bank {
            let mut rows = Vec::with_capacity(64);
            for pattern in &bank.0 {
                if !pattern.valid() {
                    return Err(Error::Pattern);
                }
                rows.push((
                    pattern.len,
                    pattern.steps.map(|s| {
                        [
                            match s.kind {
                                Kind::Rest => 0,
                                Kind::Note => 1,
                                Kind::Tie => 2,
                            },
                            s.key,
                            s.velocity,
                            s.gate,
                            s.ratchet,
                            s.accent as u8,
                            s.glide as u8,
                        ]
                    }),
                ));
            }
            Some(rows)
        } else {
            None
        };
        let payload = Payload {
            name: self.name.clone(),
            category: self.category.clone(),
            values: s.values.to_vec(),
            glide: s.glide,
            drive_makeup: s.drive_makeup,
            reverb: [s.algorithm as u8, 1],
            enabled: s.reverb_enabled,
            kill_tail: s.kill_tail,
            oscillator: [p.topology as u8, 1],
            routes: p.routes.map(|r| r as u8),
            ratios: p.ratios,
            xy: xy(p.position),
            stages: p.stages.map(|s| (xy(s.target), s.millis)),
            loop_held: p.loop_held,
            patterns,
        };
        let crc32 = crc(&serde_json::to_vec(&payload).map_err(|_| Error::Json)?);
        let data = serde_json::to_vec(&Native {
            format: "PROPHET-XII".into(),
            version: 2,
            payload,
            crc32,
        })
        .map_err(|_| Error::Json)?;
        if data.len() > MAX_BYTES {
            return Err(Error::Size);
        }
        Ok(data)
    }
    pub fn decode(data: &[u8]) -> Result<Self, Error> {
        if data.len() > MAX_BYTES {
            return Err(Error::Size);
        }
        let header: Header = serde_json::from_slice(data).map_err(|_| Error::Json)?;
        if header.format != "PROPHET-XII" {
            return Err(Error::Version);
        }
        let legacy = |values: Vec<f32>| {
            SoundState::from_rc1(values.try_into().map_err(|_| Error::Parameters)?)
                .map_err(|_| Error::Parameters)
        };
        if header.version == 1 {
            let p: Legacy = serde_json::from_slice(data).map_err(|_| Error::Json)?;
            debug_assert_eq!((p.format.as_str(), p.version), ("PROPHET-XII", 1));
            return Ok(Self {
                name: p.name,
                category: p.category,
                sound: legacy(p.values)?,
                bank: None,
            });
        }
        if header.version != 2 {
            return Err(Error::Version);
        }
        let doc: Native = serde_json::from_slice(data).map_err(|_| Error::Json)?;
        let p = doc.payload;
        if crc(&serde_json::to_vec(&p).map_err(|_| Error::Json)?) != doc.crc32 {
            return Err(Error::Checksum);
        }
        let mut sound = legacy(p.values)?;
        sound.algorithm = match p.reverb {
            [0, 1] => Algorithm::LegacyRC1,
            [1, 1] => Algorithm::DiffuseV1,
            _ => return Err(Error::Algorithm),
        };
        let topology = match p.oscillator {
            [0, 1] => Topology::Classic2,
            [1, 1] => Topology::Extended3,
            [2, 1] => Topology::Extended4,
            [3, 1] => Topology::Vector4,
            _ => return Err(Error::Algorithm),
        };
        let route = |v: u8| {
            [
                Route::Sum,
                Route::Frequency,
                Route::Phase,
                Route::Sync,
                Route::Ring,
            ]
            .get(v as usize)
            .copied()
            .ok_or(Error::Algorithm)
        };
        sound.patch = Patch {
            topology,
            routes: [route(p.routes[0])?, route(p.routes[1])?],
            ratios: p.ratios,
            position: point(p.xy),
            stages: p.stages.map(|(pos, millis)| Stage {
                target: point(pos),
                millis,
            }),
            loop_held: p.loop_held,
        };
        sound.glide = p.glide;
        sound.drive_makeup = p.drive_makeup;
        sound.kill_tail = p.kill_tail;
        sound.reverb_enabled = p.enabled;
        sound.validate().map_err(|_| Error::Parameters)?;
        let bank = if let Some(rows) = p.patterns {
            if rows.len() != 64 {
                return Err(Error::Pattern);
            }
            let mut bank = Box::<PatternBank>::default();
            for ((len, steps), dest) in rows.into_iter().zip(&mut bank.0) {
                let mut pattern = Pattern {
                    len,
                    ..Default::default()
                };
                for (s, [kind, key, velocity, gate, ratchet, accent, glide]) in
                    pattern.steps.iter_mut().zip(steps)
                {
                    if accent > 1 || glide > 1 {
                        return Err(Error::Pattern);
                    }
                    *s = Step {
                        kind: match kind {
                            0 => Kind::Rest,
                            1 => Kind::Note,
                            2 => Kind::Tie,
                            _ => return Err(Error::Pattern),
                        },
                        key,
                        velocity,
                        gate,
                        ratchet,
                        accent: accent != 0,
                        glide: glide != 0,
                    };
                }
                if !pattern.valid() {
                    return Err(Error::Pattern);
                }
                *dest = pattern;
            }
            Some(bank)
        } else {
            None
        };
        Ok(Self {
            name: p.name,
            category: p.category,
            sound,
            bank,
        })
    }
}
