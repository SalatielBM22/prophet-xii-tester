//! Linear amplitude mix on a diamond. Stage 3 sustains (or loops to 0);
//! key release starts stage 4 from the current position.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}
impl Point {
    pub fn valid(self) -> bool {
        self.x.abs() + self.y.abs() <= 1.0
    }
    pub fn weights(self) -> [f32; 4] {
        let c = ((1.0 - self.x.abs() - self.y.abs()) * 0.25).max(0.);
        [
            c + (-self.x).max(0.),
            c + self.y.max(0.),
            c + self.x.max(0.),
            c + (-self.y).max(0.),
        ]
    }
    fn plus(self, other: Self) -> Self {
        let (x, y) = (self.x + other.x, self.y + other.y);
        let scale = (x.abs() + y.abs()).max(1.);
        Self {
            x: x / scale,
            y: y / scale,
        }
    }
    fn towards(&mut self, to: Self, step: f32) {
        self.x += (to.x - self.x) * step;
        self.y += (to.y - self.y) * step;
    }
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stage {
    pub target: Point,
    pub millis: u16,
}
impl Default for Stage {
    fn default() -> Self {
        Self {
            target: Point::default(),
            millis: 100,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Topology {
    #[default]
    Classic2,
    Extended3,
    Extended4,
    Vector4,
}
/// C is modulated by B, D by C. Forward edges prevent cycles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Route {
    #[default]
    Sum,
    Frequency,
    Phase,
    Sync,
    Ring,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Patch {
    pub topology: Topology,
    pub routes: [Route; 2],
    pub ratios: [f32; 2],
    pub position: Point,
    pub stages: [Stage; 5],
    pub loop_held: bool,
}
impl Default for Patch {
    fn default() -> Self {
        Self {
            topology: Topology::Classic2,
            routes: [Route::Sum; 2],
            ratios: [0.5, 1.5],
            position: Point::default(),
            stages: [Stage::default(); 5],
            loop_held: false,
        }
    }
}
impl Patch {
    pub fn valid(&self) -> bool {
        self.position.valid()
            && self.stages.iter().all(|s| s.target.valid() && s.millis > 0)
            && self.ratios.iter().all(|v| (0.0625..=16.).contains(v))
    }
}
#[derive(Clone, Debug, Default)]
pub struct Trajectory {
    pos: Point,
    joystick: Point,
    index: usize,
    left: u32,
    finished: bool,
}
impl Trajectory {
    pub fn restart(&mut self) {
        self.index = 0;
        self.left = 0;
        self.finished = false;
    }
    pub fn release(&mut self) {
        if self.index != 4 {
            self.index = 4;
            self.left = 0;
            self.finished = false;
        }
    }
    pub fn advance(&mut self, patch: &Patch, rate: f32) -> [f32; 4] {
        if !self.finished {
            let stage = patch.stages[self.index];
            if self.left == 0 {
                self.left = (rate * (stage.millis as f32 * 0.001) + 0.5) as u32;
            }
            self.pos.towards(stage.target, 1. / self.left.max(1) as f32);
            self.left = self.left.saturating_sub(1);
            if self.left == 0 {
                self.pos = stage.target;
                match self.index {
                    0..=2 => self.index += 1,
                    3 if patch.loop_held => self.index = 0,
                    _ => self.finished = true,
                }
            }
        }
        self.joystick
            .towards(patch.position, 1. / (0.005 * rate).max(1.));
        self.pos.plus(self.joystick).weights()
    }
}
