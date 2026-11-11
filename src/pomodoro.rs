//! Pomodoro timer logic: a state machine with no UI code.
//!
//! Every method that depends on time takes `now` instead of reading the clock,
//! so tests can jump ahead without sleeping. The behaviour per state and input
//! is the table in README.md (## Pomodoro).

use std::time::{Duration, Instant};

// Shortest and longest the knob can adjust a Focus or Break to.
const MIN_LEN: Duration = Duration::from_secs(60);
const MAX_LEN: Duration = Duration::from_secs(99 * 60);

/// What a knob turn on an alarm screen starts ("5 more minutes").
const EXTEND: Duration = Duration::from_secs(5 * 60);

/// One focus/break pair, in minutes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Preset {
    pub focus_min: u32,
    pub break_min: u32,
}

/// The presets a knob turn cycles through on the Ready screen.
pub const PRESETS: [Preset; 3] = [
    Preset { focus_min: 25, break_min: 5 },
    Preset { focus_min: 50, break_min: 10 },
    Preset { focus_min: 15, break_min: 3 },
];

/// Stores a deadline rather than a counter ticked down every second, so it can't drift.
/// `Instant` is monotonic: unaffected when the device's wall clock jumps (no NTP).
#[derive(Debug, Clone, Copy)]
enum Timer {
    Running { ends_at: Instant },
    Paused  { left: Duration },
}

/// The running part of a Focus or Break; both behave the same.
#[derive(Debug)]
struct Countdown {
    timer: Timer,
    /// Full length of this phase, for the progress ring. Changes when the knob adjusts it.
    total: Duration,
}

/// Each variant carries only the data it has, so e.g. "Ready with a running countdown"
/// can't exist.
#[derive(Debug)]
enum State {
    Ready,
    Focus(Countdown),
    FocusDone,
    Break(Countdown),
    BreakDone,
}

#[derive(Debug)]
pub struct Pomodoro {
    state: State,
    preset: usize,
}

/// Which screen to show: `State` without its data, so `State` can stay private.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Phase { Ready, Focus, FocusDone, Break, BreakDone }

/// Snapshot of everything the screen needs, computed for one moment by [`Pomodoro::view`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub phase: Phase,
    pub paused: bool,
    /// Index into [`PRESETS`].
    pub preset: usize,
    /// Rounded up: 25:00 at the start, 0:00 only at the very end.
    pub remaining_secs: u32,
    /// Fraction elapsed, 0.0 → 1.0.
    pub progress: f32,
    pub focus_min: u32,
    pub break_min: u32,
}

impl Countdown {
    /// A running countdown of `total`, starting at `now`.
    fn new(total: Duration, now: Instant) -> Self {
        Countdown {
            timer: Timer::Running { ends_at: now + total },
            total,
        }
    }

    /// Running → Paused (remember what's left), Paused → Running (new deadline from `now`).
    fn toggle_pause(&mut self, now: Instant) {
        self.timer = match self.timer {
            Timer::Running { .. } => Timer::Paused { left: self.remaining(now) },
            Timer::Paused { left } => Timer::Running { ends_at: now + left },
        };
    }

    /// Add or remove whole minutes. Changes both the remaining time and `total`,
    /// and keeps the remaining time within MIN_LEN..=MAX_LEN.
    fn adjust(&mut self, minutes: i32, now: Instant) {
        let left = self.remaining(now);
        // Duration can't be negative, so work with the size of the step and its sign separately.
        let step = Duration::from_secs(60 * u64::from(minutes.unsigned_abs()));
        let new_left = if minutes >= 0 {
            // `.max(left)`: if already above MAX_LEN, turning up never shortens it.
            (left + step).min(MAX_LEN.max(left))
        } else {
            // `.min(left)`: at 0:30, turning down stays at 0:30 instead of jumping up to 1:00.
            left.saturating_sub(step).max(MIN_LEN.min(left))
        };

        // Keep the time already spent the same, so the ring doesn't jump backwards.
        let elapsed = self.total.saturating_sub(left);
        self.total = elapsed + new_left;

        self.timer = match self.timer {
            Timer::Running { .. } => Timer::Running { ends_at: now + new_left },
            Timer::Paused { .. } => Timer::Paused { left: new_left },
        };
    }

    /// Time left at `now`; zero once the deadline has passed.
    fn remaining(&self, now: Instant) -> Duration {
        match self.timer {
            Timer::Running { ends_at } => ends_at.saturating_duration_since(now),
            Timer::Paused { left } => left,
        }
    }

    /// Fraction elapsed at `now`: 0.0 at the start, 1.0 at the deadline.
    fn progress(&self, now: Instant) -> f32 {
        debug_assert!(!self.total.is_zero(), "every countdown is at least MIN_LEN");
        let left = self.remaining(now).as_secs_f32() / self.total.as_secs_f32();
        (1.0 - left).clamp(0.0, 1.0)
    }

    fn is_paused(&self) -> bool {
        matches!(self.timer, Timer::Paused { .. })
    }

    /// True once a running countdown has reached zero (a paused one never finishes).
    fn is_finished(&self, now: Instant) -> bool {
        matches!(self.timer, Timer::Running { ends_at } if now >= ends_at)
    }
}

impl Pomodoro {
    /// Starts on Ready with the first preset.
    pub fn new() -> Self {
        Pomodoro { state: State::Ready, preset: 0 }
    }

    // Each event method below follows the same shape: first `tick(now)`, so a countdown
    // that has run out becomes its alarm before the event sees it. Then move the old
    // state out with `mem::replace` (leaving a temporary Ready), match on it by value,
    // and store the state the match returns. No `_ =>` arms, so adding a State variant
    // breaks the build until every event handles it.

    /// Knob press. See the table in README.md.
    pub fn press(&mut self, now: Instant) {
        self.tick(now);
        self.state = match std::mem::replace(&mut self.state, State::Ready) {
            State::Ready => State::Focus(Countdown::new(self.focus_len(), now)),
            State::Focus(mut c) => {
                c.toggle_pause(now);
                State::Focus(c)
            }
            State::FocusDone => State::Break(Countdown::new(self.break_len(), now)),
            State::Break(_) => State::Ready, // skip break
            State::BreakDone => State::Focus(Countdown::new(self.focus_len(), now)),
        };
    }

    /// Knob turn: +1 = clockwise, -1 = anticlockwise (may be bigger on fast spins).
    pub fn turn(&mut self, delta: i32, now: Instant) {
        if delta == 0 {
            return; // otherwise an alarm screen would treat "no turn" as "5 more minutes"
        }
        self.tick(now);
        self.state = match std::mem::replace(&mut self.state, State::Ready) {
            State::Ready => {
                // rem_euclid wraps negatives (-1 → 2) and is always 0..3, so the cast is safe.
                let step = delta.rem_euclid(PRESETS.len() as i32) as usize;
                self.preset = (self.preset + step) % PRESETS.len();
                State::Ready
            }
            State::Focus(mut c) => {
                c.adjust(delta, now);
                State::Focus(c)
            }
            State::FocusDone => State::Focus(Countdown::new(EXTEND, now)),
            State::Break(mut c) => {
                c.adjust(delta, now);
                State::Break(c)
            }
            State::BreakDone => State::Break(Countdown::new(EXTEND, now)),
        };
    }

    /// Back button: always returns to Ready.
    pub fn back(&mut self) {
        self.state = State::Ready;
    }

    /// Focus → FocusDone, Break → BreakDone once the deadline has passed.
    /// Called often by a UI timer, and first thing by every event.
    pub fn tick(&mut self, now: Instant) {
        self.state = match std::mem::replace(&mut self.state, State::Ready) {
            s @ (State::Ready | State::FocusDone | State::BreakDone) => s,
            State::Focus(c) => {
                if c.is_finished(now) { State::FocusDone } else { State::Focus(c) }
            }
            State::Break(c) => {
                if c.is_finished(now) { State::BreakDone } else { State::Break(c) }
            }
        };
    }

    /// Snapshot of everything the screen needs at `now`.
    pub fn view(&self, now: Instant) -> View {
        let (phase, paused, remaining, progress) = match &self.state {
            State::Ready => (Phase::Ready, false, self.focus_len(), 0.0),
            State::Focus(c) => (Phase::Focus, c.is_paused(), c.remaining(now), c.progress(now)),
            State::FocusDone => (Phase::FocusDone, false, Duration::ZERO, 1.0),
            State::Break(c) => (Phase::Break, c.is_paused(), c.remaining(now), c.progress(now)),
            State::BreakDone => (Phase::BreakDone, false, Duration::ZERO, 1.0),
        };
        let preset = &PRESETS[self.preset];
        View {
            phase,
            paused,
            preset: self.preset,
            // At most MAX_LEN (5940 s), so the cast can't truncate.
            remaining_secs: remaining.as_millis().div_ceil(1000) as u32,
            progress,
            focus_min: preset.focus_min,
            break_min: preset.break_min,
        }
    }

    /// Focus length of the current preset.
    fn focus_len(&self) -> Duration {
        Duration::from_secs(60 * u64::from(PRESETS[self.preset].focus_min))
    }

    /// Break length of the current preset.
    fn break_len(&self) -> Duration {
        Duration::from_secs(60 * u64::from(PRESETS[self.preset].break_min))
    }
}

impl Default for Pomodoro {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn ready_shows_preset_and_turn_wraps() {
        let mut p = Pomodoro::new();
        let t0 = Instant::now();
        assert_eq!(p.view(t0).remaining_secs, 25 * 60);
        p.turn(-1, t0); // wraps from the first preset to the last
        assert_eq!(p.view(t0).preset, 2);
        assert_eq!(p.view(t0).remaining_secs, 15 * 60);
    }

    #[test]
    fn full_cycle() {
        let mut p = Pomodoro::new();
        let t0 = Instant::now();
        p.press(t0);
        assert_eq!(p.view(t0 + secs(10)).remaining_secs, 25 * 60 - 10);

        let t1 = t0 + secs(25 * 60);
        p.tick(t1);
        assert_eq!(p.view(t1).phase, Phase::FocusDone);

        p.press(t1);
        assert_eq!(p.view(t1).phase, Phase::Break);
        assert_eq!(p.view(t1).remaining_secs, 5 * 60);

        let t2 = t1 + secs(5 * 60);
        p.tick(t2);
        assert_eq!(p.view(t2).phase, Phase::BreakDone);

        p.press(t2);
        assert_eq!(p.view(t2).phase, Phase::Focus);
    }

    #[test]
    fn pause_freezes_time() {
        let mut p = Pomodoro::new();
        let t0 = Instant::now();
        p.press(t0);
        p.press(t0 + secs(10)); // pause
        let later = t0 + secs(3600);
        p.tick(later); // a paused countdown never finishes
        let v = p.view(later);
        assert!(v.paused);
        assert_eq!(v.remaining_secs, 25 * 60 - 10);
        p.press(later); // resume
        assert_eq!(p.view(later + secs(5)).remaining_secs, 25 * 60 - 15);
    }

    #[test]
    fn turn_adjusts_and_clamps() {
        let mut p = Pomodoro::new();
        let t0 = Instant::now();
        p.press(t0);
        p.turn(2, t0);
        assert_eq!(p.view(t0).remaining_secs, 27 * 60);
        p.turn(-100, t0);
        assert_eq!(p.view(t0).remaining_secs, 60); // floor is 1 min

        // Under a minute left: turning down must not jump back up to 1:00.
        let t1 = t0 + secs(30);
        p.turn(-1, t1);
        assert_eq!(p.view(t1).remaining_secs, 30);
    }

    #[test]
    fn adjust_keeps_progress_from_jumping_back() {
        let mut p = Pomodoro::new();
        let t0 = Instant::now();
        p.press(t0);
        let t1 = t0 + secs(5 * 60);
        p.turn(5, t1); // 5 min elapsed of now 30 min total
        assert!((p.view(t1).progress - 5.0 / 30.0).abs() < 1e-3);
    }

    #[test]
    fn alarm_turn_extends_by_five_minutes() {
        let mut p = Pomodoro::new();
        let t0 = Instant::now();
        p.press(t0);
        let t1 = t0 + secs(25 * 60);
        p.tick(t1);
        p.turn(1, t1);
        let v = p.view(t1);
        assert_eq!(v.phase, Phase::Focus);
        assert_eq!(v.remaining_secs, 5 * 60);
    }

    #[test]
    fn press_after_deadline_acts_on_the_alarm() {
        // No tick between the deadline and the press: the press must still start the break.
        let mut p = Pomodoro::new();
        let t0 = Instant::now();
        p.press(t0);
        let t1 = t0 + secs(25 * 60);
        p.press(t1);
        assert_eq!(p.view(t1).phase, Phase::Break);
    }

    #[test]
    fn skip_break_and_back_go_to_ready() {
        let mut p = Pomodoro::new();
        let t0 = Instant::now();
        p.press(t0);
        p.back();
        assert_eq!(p.view(t0).phase, Phase::Ready);

        p.press(t0);
        p.tick(t0 + secs(25 * 60));
        p.press(t0 + secs(25 * 60)); // start break
        p.press(t0 + secs(25 * 60)); // skip it
        assert_eq!(p.view(t0).phase, Phase::Ready);
    }
}