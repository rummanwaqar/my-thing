//! When to dim the screen: after DIM_AFTER without input on the Ready screen.
//!
//! No UI or sysfs code. Like `pomodoro.rs`, every method takes `now`, so tests
//! can jump ahead without sleeping.

use std::time::{Duration, Instant};

/// How long the Ready screen waits without input before dimming.
pub const DIM_AFTER: Duration = Duration::from_secs(20);

#[derive(Debug)]
pub struct Idle {
    last_input: Instant,
    dimmed: bool,
}

impl Idle {
    /// Starts bright, as if there was input at `now`.
    pub fn new(now: Instant) -> Self {
        Idle { last_input: now, dimmed: false }
    }

    /// Any input. Restarts the countdown; returns true if the screen must be brightened.
    pub fn input(&mut self, now: Instant) -> bool {
        self.last_input = now;
        std::mem::replace(&mut self.dimmed, false)
    }

    /// Called often by the UI timer. Returns `Some(dimmed)` only when that changes:
    /// dims on Ready after DIM_AFTER without input, brightens when not on Ready.
    pub fn tick(&mut self, now: Instant, on_ready: bool) -> Option<bool> {
        let want = on_ready && now.duration_since(self.last_input) >= DIM_AFTER;
        if want == self.dimmed {
            return None;
        }
        self.dimmed = want;
        Some(want)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn dims_after_20s_on_ready() {
        let t0 = Instant::now();
        let mut idle = Idle::new(t0);
        assert_eq!(idle.tick(t0 + secs(19), true), None);
        assert_eq!(idle.tick(t0 + secs(20), true), Some(true));
        assert_eq!(idle.tick(t0 + secs(21), true), None); // only reports the change
    }

    #[test]
    fn never_dims_outside_ready() {
        let t0 = Instant::now();
        let mut idle = Idle::new(t0);
        assert_eq!(idle.tick(t0 + secs(3600), false), None);
    }

    #[test]
    fn input_restarts_countdown() {
        let t0 = Instant::now();
        let mut idle = Idle::new(t0);
        assert!(!idle.input(t0 + secs(15))); // not dimmed: nothing to brighten
        assert_eq!(idle.tick(t0 + secs(30), true), None);
        assert_eq!(idle.tick(t0 + secs(35), true), Some(true));
    }

    #[test]
    fn input_wakes_when_dimmed() {
        let t0 = Instant::now();
        let mut idle = Idle::new(t0);
        idle.tick(t0 + secs(20), true);
        assert!(idle.input(t0 + secs(25)));
        assert_eq!(idle.tick(t0 + secs(26), true), None); // stays bright
        assert_eq!(idle.tick(t0 + secs(45), true), Some(true));
    }

    #[test]
    fn leaving_ready_while_dimmed_brightens() {
        let t0 = Instant::now();
        let mut idle = Idle::new(t0);
        idle.tick(t0 + secs(20), true);
        assert_eq!(idle.tick(t0 + secs(21), false), Some(false));
    }
}
