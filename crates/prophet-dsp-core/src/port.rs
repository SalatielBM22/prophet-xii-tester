//! Source-level Rust port. Serialize scalars explicitly at host boundaries.
pub const VERSION: u16 = 1;
pub const MAX_EVENTS: usize = 256;
pub const ORIGINS: usize = 16;
pub const GLIDE_ENABLED: u16 = 83;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prepare {
    pub sample_rate: u32,
    pub max_frames: u16,
    pub oversampling: u8,
}
impl Prepare {
    pub fn valid(self) -> bool {
        (8000..=192000).contains(&self.sample_rate)
            && (1..=1024).contains(&self.max_frames)
            && matches!(self.oversampling, 1 | 2 | 4)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    Midi {
        status: u8,
        data1: u8,
        data2: u8,
        note_id: u32,
    },
    Parameter {
        id: u16,
        value: f32,
    },
    VectorXY {
        x: f32,
        y: f32,
    },
    PanicOrigin,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Command {
    pub offset: u16,
    pub origin: u8,
    pub serial: u32,
    pub action: Action,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    Applied,
    Duplicate,
    Capacity,
    Unsupported,
    #[default]
    Invalid,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ack {
    pub origin: u8,
    pub serial: u32,
    pub revision: u64,
    pub status: Status,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PortError {
    Configuration,
    Block,
    Offset,
    Origin,
    Capacity,
}
pub trait EnginePort: Sized {
    fn prepare(config: Prepare) -> Result<Self, PortError>;
    fn process(
        &mut self,
        left: &mut [f32],
        right: &mut [f32],
        events: &[Command],
        acks: &mut [Ack],
    ) -> Result<(), PortError>;
}
pub fn validate_block(
    config: Prepare,
    left: usize,
    right: usize,
    events: &[Command],
    acks: usize,
) -> Result<(), PortError> {
    if left != right || left > usize::from(config.max_frames) {
        return Err(PortError::Block);
    }
    if events.len() > MAX_EVENTS || acks < events.len() {
        return Err(PortError::Capacity);
    }
    let mut previous = 0;
    for event in events {
        if event.offset < previous || usize::from(event.offset) >= left {
            return Err(PortError::Offset);
        }
        if usize::from(event.origin) >= ORIGINS || event.serial == 0 {
            return Err(PortError::Origin);
        }
        previous = event.offset;
    }
    Ok(())
}
