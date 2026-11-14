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
use calendar::{Agenda, CalState, Row, RowKind};
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
    ui.set_today_label(today_label().into());

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
    // Calendar: turn scrolls the agenda, press syncs now, Back jumps to now.
    ui.on_knob_turned({
        let (ui, pomo, idle, page, cal) = (ui.as_weak(), pomo.clone(), idle.clone(), page.clone(), cal.clone());
        move |delta| {
            wake(&idle);
            match page.get() {
                Page::Pomodoro => apply(&ui, &pomo, |p, now| p.turn(delta, now)),
                Page::Calendar => cal_action(&ui, &cal, |c| c.agenda.scroll(delta)),
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
                Page::Calendar => cal_action(&ui, &cal, |c| c.agenda.back_to_now()),
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
                cal_action(&ui, &cal, |c| c.agenda.back_to_now());
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
                if dimmed {
                    cal.borrow_mut().agenda.back_to_now(); // nobody's scrolling any more
                }
            }
            // Cheap, and catches midnight without a separate timer.
            if let Some(ui) = ui.upgrade() {
                let label = today_label();
                if ui.get_today_label() != label.as_str() {
                    ui.set_today_label(label.into());
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

/// Today's date for the Calendar placeholder, e.g. "Saturday, 14 November".
fn today_label() -> String {
    chrono::Local::now().format("%A, %-d %B").to_string()
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

/// Rows handed to Slint: about what fits on the 480 px screen, plus one.
const VISIBLE_ROWS: usize = 9;

/// Everything the Calendar page shows, on the UI thread.
#[derive(Default)]
struct CalendarUi {
    data: CalState,
    agenda: Agenda,
    /// Login state from the Google thread.
    status: Option<Status>,
    /// The rows last given to Slint, to skip unchanged updates.
    pushed: Vec<Row>,
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

    /// Rebuilds the agenda for now and sets the Slint properties that changed.
    /// The only place calendar types turn into Slint types.
    fn push(&mut self, ui: &MainWindow) {
        self.agenda.refresh(&self.data.events, chrono::Local::now());
        let (ready, line) = match &self.status {
            Some(Status::LoggedIn { email }) => {
                (self.data.last_sync.is_some(), format!("{} · {email}", self.data.sync_label()))
            }
            Some(s) => (false, s.label()),
            None => (false, String::new()),
        };
        if ui.get_google_status() != line.as_str() {
            ui.set_google_status(line.into());
        }
        if ui.get_calendar_ready() != ready {
            ui.set_calendar_ready(ready);
        }
        let visible = self.agenda.visible(VISIBLE_ROWS);
        if visible != self.pushed.as_slice() {
            self.pushed = visible.to_vec();
            let rows: Vec<CalRow> = visible.iter().map(cal_row).collect();
            ui.set_calendar_rows(ModelRc::new(VecModel::from(rows)));
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

fn cal_row(r: &Row) -> CalRow {
    CalRow {
        kind: match r.kind {
            RowKind::Day => CalRowKind::Day,
            RowKind::Event => CalRowKind::Event,
            RowKind::Empty => CalRowKind::Empty,
        },
        label: r.label.as_str().into(),
        title: r.title.as_str().into(),
        detail: r.detail.as_str().into(),
        color: r.color.as_deref().and_then(hex_color).unwrap_or(slint::Color::from_rgb_u8(0x88, 0x88, 0x88)),
        now: r.now,
        past: r.past,
    }
}

/// "#9fe1e7" → a Slint color.
fn hex_color(s: &str) -> Option<slint::Color> {
    let hex = s.strip_prefix('#').filter(|h| h.len() == 6)?;
    let n = u32::from_str_radix(hex, 16).ok()?;
    Some(slint::Color::from_rgb_u8((n >> 16) as u8, (n >> 8) as u8, n as u8))
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
