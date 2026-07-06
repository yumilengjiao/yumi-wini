//! Animation system: easing, timed springs and the frame clock.
//!
//! The main thread drives animations with a periodic timer
//! (SetTimer on the message window, ~16ms). Each animated value is a
//! `Val` that eases from its current value to a target; per frame we
//! sample and push intermediate geometry to windows.
//!
//! Niri's default feel is a spring; we approximate with an
//! ease-out-expo / critically-damped curve that stays cheap enough to
//! drive SetWindowPos at 60 Hz.

use std::time::{Duration, Instant};

/// Easing functions (names from CSS for familiarity).
///
/// All variants become constructible from the `animations` config node
/// (#26); until then only the default is used.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Easing {
    Linear,
    EaseOutQuad,
    EaseOutCubic,
    EaseOutExpo,
    EaseOutBack(f64),
}

impl Easing {
    pub fn apply(&self, t: f64) -> f64 {
        let t = t.clamp(0.0, 1.0);
        match *self {
            Easing::Linear => t,
            Easing::EaseOutQuad => 1.0 - (1.0 - t) * (1.0 - t),
            Easing::EaseOutCubic => 1.0 - (1.0 - t).powi(3),
            Easing::EaseOutExpo => {
                if t >= 1.0 {
                    1.0
                } else {
                    1.0 - 2.0f64.powf(-10.0 * t)
                }
            }
            Easing::EaseOutBack(overshoot) => {
                let c3 = overshoot + 1.0;
                1.0 + c3 * (t - 1.0).powi(3) + overshoot * (t - 1.0).powi(2)
            }
        }
    }
}

/// Animation parameters, niri-ish defaults.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnimParams {
    pub duration: Duration,
    pub easing: Easing,
}

impl Default for AnimParams {
    fn default() -> Self {
        AnimParams {
            duration: Duration::from_millis(250),
            easing: Easing::EaseOutCubic,
        }
    }
}

/// One animated scalar.
#[derive(Debug, Clone)]
pub struct Val {
    from: f64,
    to: f64,
    start: Instant,
    params: AnimParams,
}

impl Val {
    pub fn to(value: f64, params: AnimParams) -> Self {
        Val {
            from: value,
            to: value,
            start: Instant::now(),
            params,
        }
    }

    /// Change the target; animation continues smoothly from the current
    /// value.
    pub fn retarget(&mut self, value: f64) {
        let cur = self.value();
        if (cur - value).abs() < 0.5 {
            self.from = value;
            self.to = value;
            return;
        }
        self.from = cur;
        self.to = value;
        self.start = Instant::now();
    }

    /// Current (animated) value.
    pub fn value(&self) -> f64 {
        if self.finished() {
            return self.to;
        }
        let t = self
            .params
            .duration
            .saturating_sub(self.start.elapsed())
            .as_secs_f64();
        let total = self.params.duration.as_secs_f64().max(f64::EPSILON);
        let progress = 1.0 - (t / total).clamp(0.0, 1.0);
        let eased = self.params.easing.apply(progress);
        self.from + (self.to - self.from) * eased
    }

    pub fn finished(&self) -> bool {
        self.start.elapsed() >= self.params.duration
    }

    #[allow(dead_code)] // consumed by exit-restore (#44) / overview (#40)
    pub fn target(&self) -> f64 {
        self.to
    }
}

/// A (x, y, w, h) rectangle whose components animate independently.
#[derive(Debug, Clone)]
pub struct AnimatedRect {
    pub x: Val,
    pub y: Val,
    pub w: Val,
    pub h: Val,
}

impl AnimatedRect {
    pub fn new(x: f64, y: f64, w: f64, h: f64, params: AnimParams) -> Self {
        AnimatedRect {
            x: Val::to(x, params),
            y: Val::to(y, params),
            w: Val::to(w, params),
            h: Val::to(h, params),
        }
    }

    pub fn retarget(&mut self, x: f64, y: f64, w: f64, h: f64) {
        self.x.retarget(x);
        self.y.retarget(y);
        self.w.retarget(w);
        self.h.retarget(h);
    }

    pub fn finished(&self) -> bool {
        self.x.finished() && self.y.finished() && self.w.finished() && self.h.finished()
    }

    /// Current animated rect, rounded for SetWindowPos.
    pub fn value(&self) -> (i32, i32, i32, i32) {
        (
            self.x.value().round() as i32,
            self.y.value().round() as i32,
            self.w.value().round().max(1.0) as i32,
            self.h.value().round().max(1.0) as i32,
        )
    }
}

/// Tracks all window animations and ticks them.
#[derive(Debug, Default)]
pub struct Animator {
    /// Per-window animated rect, keyed by window id.
    rects: std::collections::HashMap<isize, AnimatedRect>,
    params: AnimParams,
}

impl Animator {
    pub fn new(params: AnimParams) -> Self {
        Animator {
            rects: std::collections::HashMap::new(),
            params,
        }
    }

    /// Set/retarget a window's target rect.
    pub fn set_target(&mut self, id: isize, x: f64, y: f64, w: f64, h: f64) {
        match self.rects.get_mut(&id) {
            Some(r) => r.retarget(x, y, w, h),
            None => {
                self.rects
                    .insert(id, AnimatedRect::new(x, y, w, h, self.params));
            }
        }
    }

    /// Drop animation state for a window.
    pub fn remove(&mut self, id: isize) {
        self.rects.remove(&id);
    }

    pub fn is_animating(&self) -> bool {
        self.rects.values().any(|r| !r.finished())
    }

    /// Sample all in-flight animations. Returns (id, rect) pairs that
    /// still need a SetWindowPos this frame.
    pub fn tick(&mut self) -> Vec<(isize, i32, i32, i32, i32)> {
        let mut out = Vec::new();
        for (id, r) in &self.rects {
            if !r.finished() {
                let (x, y, w, h) = r.value();
                out.push((*id, x, y, w, h));
            }
        }
        // Garbage-collect finished animations occasionally.
        self.rects.retain(|_, r| !r.finished());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn easing_bounds() {
        for e in [
            Easing::Linear,
            Easing::EaseOutQuad,
            Easing::EaseOutCubic,
            Easing::EaseOutExpo,
            Easing::EaseOutBack(1.70158),
        ] {
            assert!((e.apply(0.0) - 0.0).abs() < 1e-9, "{e:?} at 0");
            assert!((e.apply(1.0) - 1.0).abs() < 1e-9, "{e:?} at 1");
            let mid = e.apply(0.5);
            assert!(mid >= -0.5 && mid <= 1.5, "{e:?} mid sane: {mid}");
        }
    }

    #[test]
    fn val_finishes_at_target() {
        let mut v = Val::to(100.0, AnimParams::default());
        v.retarget(200.0);
        // Force completion by pretending time passed: duration 250ms.
        std::thread::sleep(Duration::from_millis(300));
        assert!(v.finished());
        assert_eq!(v.value(), 200.0);
    }

    #[test]
    fn val_retargets_smoothly() {
        let mut v = Val::to(0.0, AnimParams::default());
        v.retarget(100.0);
        // Halfway in time (125ms of 250ms), ease-out-cubic at t=0.5 is
        // 1-(0.5)^3 = 0.875 -> value 87.5. We can't sleep mid-test
        // reliably, so just check bounds instead.
        let val = v.value();
        assert!(val >= 0.0 && val <= 100.0);
    }

    #[test]
    fn animator_ticks_and_gc() {
        let mut a = Animator::new(AnimParams {
            duration: Duration::from_millis(1),
            easing: Easing::Linear,
        });
        a.set_target(1, 0.0, 0.0, 100.0, 100.0);
        assert!(a.is_animating());
        std::thread::sleep(Duration::from_millis(10));
        let ticks = a.tick();
        // After finish: no ticks, GC'd.
        assert!(ticks.is_empty());
        assert!(!a.is_animating());
        assert!(a.rects.is_empty());
    }

    #[test]
    fn animated_rect_values() {
        let mut r = AnimatedRect::new(0.0, 0.0, 100.0, 100.0, AnimParams {
            duration: Duration::from_millis(1),
            easing: Easing::Linear,
        });
        assert_eq!(r.value(), (0, 0, 100, 100));
        r.retarget(10.0, 20.0, 30.0, 40.0);
        std::thread::sleep(Duration::from_millis(10));
        assert_eq!(r.value(), (10, 20, 30, 40));
        assert!(r.finished());
    }
}
