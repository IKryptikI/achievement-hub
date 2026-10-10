//! Small animation toolkit: values that glide toward targets (frame-rate
//! independent), plus easing curves.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

#[derive(Default)]
pub struct Anim {
    values: HashMap<u64, f32>,
    dt: f32,
}

fn key(k: impl Hash) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    k.hash(&mut h);
    h.finish()
}

impl Anim {
    /// Call once per frame with the frame time in seconds.
    pub fn begin(&mut self, dt: f32) {
        // Clamp so a hitch (or the first frame) doesn't snap everything.
        self.dt = dt.clamp(0.0, 0.1);
    }

    /// Forget all values: everything animates in from its start again.
    pub fn reset(&mut self) {
        self.values.clear();
    }

    /// Moves the value for `k` toward `target` and returns it. `speed` is
    /// roughly "how many times per second it closes the gap"; `start` is the
    /// value the first time the key is seen.
    pub fn to(&mut self, k: impl Hash, target: f32, speed: f32, start: f32) -> f32 {
        let v = self.values.entry(key(k)).or_insert(start);
        let t = 1.0 - (-speed * self.dt).exp();
        *v += (target - *v) * t;
        if (target - *v).abs() < 0.0005 {
            *v = target;
        }
        *v
    }

    /// Time-based 0→1 progress since `set` was last called for `k`.
    pub fn timer(&mut self, k: impl Hash, duration: f32) -> f32 {
        let v = self.values.entry(key(k)).or_insert(0.0);
        *v = (*v + self.dt / duration.max(0.001)).min(1.0);
        *v
    }

    /// Current value for `k` without advancing it (`default` if unseen).
    pub fn peek(&self, k: impl Hash, default: f32) -> f32 {
        self.values.get(&key(k)).copied().unwrap_or(default)
    }

    /// Restarts a timer (or value) at `value`.
    pub fn set(&mut self, k: impl Hash, value: f32) {
        self.values.insert(key(k), value);
    }
}

pub fn ease_out_cubic(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// Overshoots slightly, then settles (for "pop" effects).
pub fn ease_out_back(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    let c1 = 1.70158;
    let c3 = c1 + 1.0;
    1.0 + c3 * (t - 1.0).powi(3) + c1 * (t - 1.0).powi(2)
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub fn lerp_color(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    [lerp(a[0], b[0], t), lerp(a[1], b[1], t), lerp(a[2], b[2], t), lerp(a[3], b[3], t)]
}
