mod backlight;
mod calendar;
mod google;
mod idle;
mod pages;
mod pomodoro;

#[cfg(feature = "device")]
mod knob;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use idle::Idle;
use calendar::{BoardView, CalState, EventView, Pick};
use google::{Msg, Status};
use pages::Page;
use pomodoro::{PRESETS, Phase, Pomodoro, View};
use slint::{ModelRc, SharedString, VecModel};

slint::include_modules!();

// FPS: run with SLINT_DEBUG_PERFORMANCE=refresh_lazy,overlay (or console).
// Don't use Window::set_rendering_notifier: in Slint 1.18.1 it makes the
// linuxkms femtovg renderer resize its canvas to the window size after
// rotation (i-slint-renderer-femtovg lib.rs:255), shifting the whole image.

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;

    // Rc<RefCell>: several callbacks share one mutable Pomodoro, all on the UI thread.
    let pomo = Rc::new(RefCell::new(Pomodoro::new()));
    let idle = Rc::new(RefCell::new(Idle::new(Instant::now())));
    // Idle starts bright; match it in case a previous run exited while dimmed.
    backlight::set_dimmed(false);

    // Which app gets the knob / Enter / Esc. Hidden apps keep running (the tick
    // timer below always updates the Pomodoro).
    let page = Rc::new(Cell::new(Page::FIRST));
    ui.set_page(Page::FIRST.into());
    let (date, clock) = date_and_clock();
    ui.set_today_label(date.into());
    ui.set_clock_label(clock.into());

    // Google login and calendar fetching run on their own thread; the tick timer
    // drains its messages. `--logout` forgets the saved login first.
    // `--demo` skips Google and shows sample events instead.
    let logout = std::env::args().any(|a| a == "--logout");
    let (google_cmds, google_msgs) = if std::env::args().any(|a| a == "--demo") {
        demo_calendar()
    } else {
        google::spawn(logout)
    };
    let cal = Rc::new(RefCell::new(CalendarUi::default()));

    let labels: Vec<SharedString> = PRESETS
        .iter()
        .map(|p| format!("{} / {}", p.focus_min, p.break_min).into())
        .collect();
    ui.set_preset_labels(ModelRc::new(VecModel::from(labels)));

    // Each callback holds a Weak handle: a strong one would be a reference cycle
    // (the window owns the callback, the callback would own the window).
    // Every input first wakes the screen, then goes to the visible page only.
    // Calendar: turn moves the departure through upcoming events, press syncs now,
    // Back returns to the next event.
    ui.on_knob_turned({
        let (ui, pomo, idle, page, cal) = (ui.as_weak(), pomo.clone(), idle.clone(), page.clone(), cal.clone());
        move |delta| {
            wake(&idle);
            match page.get() {
                Page::Pomodoro => apply(&ui, &pomo, |p, now| p.turn(delta, now)),
                Page::Calendar => cal_action(&ui, &cal, |c| c.scroll(delta)),
            }
        }
    });
    ui.on_knob_pressed({
        let (ui, pomo, idle, page) = (ui.as_weak(), pomo.clone(), idle.clone(), page.clone());
        move || {
            wake(&idle);
            match page.get() {
                Page::Pomodoro => apply(&ui, &pomo, |p, now| p.press(now)),
                Page::Calendar => {
                    eprintln!("calendar: refresh requested");
                    let _ = google_cmds.send(google::Cmd::RefreshNow);
                }
            }
        }
    });
    ui.on_back({
        let (ui, pomo, idle, page, cal) = (ui.as_weak(), pomo.clone(), idle.clone(), page.clone(), cal.clone());
        move || {
            wake(&idle);
            match page.get() {
                Page::Pomodoro => apply(&ui, &pomo, |p, _| p.back()),
                Page::Calendar => cal_action(&ui, &cal, |c| c.back_to_next()),
            }
        }
    });
    ui.on_preset({
        let (ui, idle, page, cal) = (ui.as_weak(), idle.clone(), page.clone(), cal.clone());
        move |n| {
            wake(&idle);
            let Some(new) = Page::from_preset(n) else {
                eprintln!("preset {n}: no app yet");
                return;
            };
            page.set(new);
            if let Some(ui) = ui.upgrade() {
                ui.set_page(new.into());
            }
            if new == Page::Calendar {
                cal_action(&ui, &cal, |c| c.back_to_next());
            }
        }
    });

    // 200 ms, not 1 s: a 1 s timer drifts against the deadline, so the shown
    // seconds would visibly stall or skip.
    let tick_timer = slint::Timer::default();
    tick_timer.start(slint::TimerMode::Repeated, Duration::from_millis(200), {
        let (ui, pomo, idle, page, cal) = (ui.as_weak(), pomo.clone(), idle.clone(), page.clone(), cal.clone());
        move || {
            // Always, even while the Calendar is shown: the Pomodoro keeps running.
            apply(&ui, &pomo, |p, now| p.tick(now));
            let now = Instant::now();
            // The Calendar is a glance screen, so it dims like the Pomodoro's Ready.
            let can_dim = match page.get() {
                Page::Calendar => true,
                Page::Pomodoro => pomo.borrow().view(now).phase == Phase::Ready,
            };
            if let Some(dimmed) = idle.borrow_mut().tick(now, can_dim) {
                backlight::set_dimmed(dimmed);
            }
            // Cheap, and catches the minute / midnight without separate timers.
            if let Some(ui) = ui.upgrade() {
                let (date, clock) = date_and_clock();
                if ui.get_today_label() != date.as_str() {
                    ui.set_today_label(date.into());
                }
                if ui.get_clock_label() != clock.as_str() {
                    ui.set_clock_label(clock.into());
                }
                let mut c = cal.borrow_mut();
                while let Ok(msg) = google_msgs.try_recv() {
                    c.handle(msg);
                }
                // Every tick, so "now" and past markers move; only changes reach Slint.
                c.push(&ui);
            }
        }
    });

    push(&ui, &pomo.borrow().view(Instant::now()));

    #[cfg(feature = "device")]
    knob::spawn(ui.as_weak());

    ui.run()
}

/// Any input: restarts the idle countdown and brightens the screen if it was dimmed.
fn wake(idle: &RefCell<Idle>) {
    if idle.borrow_mut().input(Instant::now()) {
        backlight::set_dimmed(false);
    }
}

/// Runs one action on the Pomodoro, then redraws from a fresh view at the same `now`.
fn apply(
    ui: &slint::Weak<MainWindow>,
    pomo: &RefCell<Pomodoro>,
    action: impl FnOnce(&mut Pomodoro, Instant),
) {
    let now = Instant::now();
    let mut p = pomo.borrow_mut();
    action(&mut p, now);
    if let Some(ui) = ui.upgrade() {
        push(&ui, &p.view(now));
    }
}

/// The only place pomodoro types turn into Slint types.
fn push(ui: &MainWindow, v: &View) {
    ui.set_pomo(PomoView {
        phase: v.phase.into(),
        paused: v.paused,
        // Slint ints are i32; all of these are small (preset < 3, minutes ≤ 50).
        preset: v.preset as i32,
        remaining_secs: v.remaining_secs as i32,
        progress: v.progress,
        focus_min: v.focus_min as i32,
        break_min: v.break_min as i32,
    });
}

/// The Calendar header: "Wednesday, Nov 11" and "10:22".
fn date_and_clock() -> (String, String) {
    let now = chrono::Local::now();
    (now.format("%A, %b %-d").to_string(), now.format("%H:%M").to_string())
}

/// For `--demo`: a fake Google thread that is logged in and has sample events.
fn demo_calendar() -> (std::sync::mpsc::Sender<google::Cmd>, std::sync::mpsc::Receiver<Msg>) {
    let (cmd_tx, _cmd_rx) = std::sync::mpsc::channel();
    let (msg_tx, msg_rx) = std::sync::mpsc::channel();
    let now = chrono::Local::now();
    let _ = msg_tx.send(Msg::Status(Status::LoggedIn { email: "demo@example.com".into() }));
    let _ = msg_tx.send(Msg::Synced { events: calendar::sample_events(now), at: now });
    (cmd_tx, msg_rx)
}

/// How long the Calendar stays on an event the knob moved to, without input.
const BROWSE_TIMEOUT: Duration = Duration::from_secs(120);

/// Everything the Calendar page shows, on the UI thread.
#[derive(Default)]
struct CalendarUi {
    data: CalState,
    /// The upcoming event the knob moved to; None shows the actual next event.
    pick: Option<Pick>,
    /// Last knob turn on the Calendar, for going back to the next event by itself.
    browsed_at: Option<Instant>,
    /// Login state from the Google thread.
    status: Option<Status>,
    /// The board last given to Slint, to skip unchanged updates.
    pushed: Option<BoardView>,
}

impl CalendarUi {
    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Status(s) => {
                // A new login may be a different account: drop the old events.
                if s == Status::WaitingForBrowser {
                    self.data = CalState::default();
                }
                self.status = Some(s);
            }
            Msg::Synced { events, at } => {
                if self.data.apply_sync(events, at) {
                    log_upcoming(&self.data);
                }
            }
            Msg::SyncFailed(e) => self.data.apply_error(e),
        }
    }

    /// Knob turn: move the departure through the upcoming events (clamped).
    fn scroll(&mut self, delta: i32) {
        let (events, now) = (&self.data.events, chrono::Local::now());
        let max = calendar::upcoming_count(events, now).saturating_sub(1);
        let current = calendar::offset_of(events, now, self.pick.as_ref());
        let target = (current as i64 + i64::from(delta)).clamp(0, max as i64) as usize;
        self.pick = calendar::pick_at(events, now, target);
        self.browsed_at = Some(Instant::now());
    }

    fn back_to_next(&mut self) {
        self.pick = None;
        self.browsed_at = None;
    }

    /// Rebuilds the board for now and sets the Slint properties that changed.
    /// The only place calendar types turn into Slint types.
    fn push(&mut self, ui: &MainWindow) {
        let (ready, line) = match &self.status {
            // Logged in: the header only mentions sync problems.
            Some(Status::LoggedIn { .. }) => (
                self.data.last_sync.is_some(),
                if self.data.error.is_some() { self.data.sync_label() } else { String::new() },
            ),
            Some(s) => (false, s.label()),
            None => (false, String::new()),
        };
        if ui.get_google_status() != line.as_str() {
            ui.set_google_status(line.into());
        }
        if ui.get_calendar_ready() != ready {
            ui.set_calendar_ready(ready);
        }
        // Left browsing for a while: back to the next event.
        if self.browsed_at.is_some_and(|t| t.elapsed() >= BROWSE_TIMEOUT) {
            self.back_to_next();
        }
        let now = chrono::Local::now();
        let offset = calendar::offset_of(&self.data.events, now, self.pick.as_ref());
        let board = calendar::board(&self.data.events, now, offset);
        if self.pushed.as_ref() != Some(&board) {
            ui.set_calendar_board(cal_board(&board));
            self.pushed = Some(board);
        }
    }
}

/// Runs one input action on the Calendar page, then redraws it.
fn cal_action(ui: &slint::Weak<MainWindow>, cal: &RefCell<CalendarUi>, action: impl FnOnce(&mut CalendarUi)) {
    let mut c = cal.borrow_mut();
    action(&mut c);
    if let Some(ui) = ui.upgrade() {
        c.push(&ui);
    }
}

fn cal_board(b: &BoardView) -> CalBoard {
    let list = |v: &[EventView]| ModelRc::new(VecModel::from(v.iter().map(cal_event).collect::<Vec<_>>()));
    // Departures list: 4 rows of 94 px fit; the NOW row takes one.
    let departures = &b.upcoming[..b.upcoming.len().min(if b.now.is_some() { 3 } else { 4 })];
    CalBoard {
        has_now: b.now.is_some(),
        now: b.now.as_ref().map(cal_event).unwrap_or_default(),
        now_left: b.now_left.0.as_str().into(),
        now_left_unit: b.now_left.1.as_str().into(),
        now_progress: b.now_progress,
        departures: list(departures),
        all_day_count: b.all_day_count as i32,
        legend: list(&b.legend),
    }
}

fn cal_event(e: &EventView) -> CalEvent {
    let color = e.color.as_deref().and_then(hex_color).unwrap_or(slint::Color::from_rgb_u8(0x88, 0x88, 0x88));
    CalEvent {
        title: e.title.as_str().into(),
        detail: e.detail.as_str().into(),
        location: e.location.as_str().into(),
        range: e.range.as_str().into(),
        big: e.big.0.as_str().into(),
        big_unit: e.big.1.as_str().into(),
        badge: e.badge.to_string().into(),
        color,
        ink: ink_on(color),
    }
}

/// "#9fe1e7" → a Slint color.
fn hex_color(s: &str) -> Option<slint::Color> {
    let hex = s.strip_prefix('#').filter(|h| h.len() == 6)?;
    let n = u32::from_str_radix(hex, 16).ok()?;
    Some(slint::Color::from_rgb_u8((n >> 16) as u8, (n >> 8) as u8, n as u8))
}

/// Dark text on light badge colors, white on dark.
fn ink_on(c: slint::Color) -> slint::Color {
    let luma = 0.299 * f32::from(c.red()) + 0.587 * f32::from(c.green()) + 0.114 * f32::from(c.blue());
    if luma > 150.0 { slint::Color::from_rgb_u8(0x11, 0x11, 0x11) } else { slint::Color::from_rgb_u8(0xff, 0xff, 0xff) }
}

/// After a sync that changed something: the next few events, to the terminal.
fn log_upcoming(cal: &CalState) {
    eprintln!("calendar: {}", cal.sync_label());
    let now = chrono::Local::now();
    for e in cal.events.iter().filter(|e| e.end > now).take(5) {
        let when = if e.all_day {
            e.start.format("%a %d %b, all day").to_string()
        } else {
            format!("{}–{}", e.start.format("%a %d %b %H:%M"), e.end.format("%H:%M"))
        };
        eprintln!("calendar:   {when}  {}  [{}]", e.title, e.calendar);
    }
}

impl From<Page> for AppPage {
    fn from(page: Page) -> Self {
        match page {
            Page::Calendar => AppPage::Calendar,
            Page::Pomodoro => AppPage::Pomodoro,
        }
    }
}

impl From<Phase> for PomoPhase {
    fn from(phase: Phase) -> Self {
        match phase {
            Phase::Ready => PomoPhase::Ready,
            Phase::Focus => PomoPhase::Focus,
            Phase::FocusDone => PomoPhase::FocusDone,
            Phase::Break => PomoPhase::OnBreak,
            Phase::BreakDone => PomoPhase::BreakDone,
        }
    }
}
