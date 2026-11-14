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
use calendar::CalState;
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
    let logout = std::env::args().any(|a| a == "--logout");
    let (google_cmds, google_msgs) = google::spawn(logout);
    let cal = Rc::new(RefCell::new(CalState::default()));
    let mut google_status: Option<Status> = None;

    let labels: Vec<SharedString> = PRESETS
        .iter()
        .map(|p| format!("{} / {}", p.focus_min, p.break_min).into())
        .collect();
    ui.set_preset_labels(ModelRc::new(VecModel::from(labels)));

    // Each callback holds a Weak handle: a strong one would be a reference cycle
    // (the window owns the callback, the callback would own the window).
    // Every input first wakes the screen, then goes to the visible page only.
    // On the Calendar only a press does something for now: sync now.
    ui.on_knob_turned({
        let (ui, pomo, idle, page) = (ui.as_weak(), pomo.clone(), idle.clone(), page.clone());
        move |delta| {
            wake(&idle);
            match page.get() {
                Page::Pomodoro => apply(&ui, &pomo, |p, now| p.turn(delta, now)),
                Page::Calendar => {}
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
        let (ui, pomo, idle, page) = (ui.as_weak(), pomo.clone(), idle.clone(), page.clone());
        move || {
            wake(&idle);
            match page.get() {
                Page::Pomodoro => apply(&ui, &pomo, |p, _| p.back()),
                Page::Calendar => {}
            }
        }
    });
    ui.on_preset({
        let (ui, idle, page) = (ui.as_weak(), idle.clone(), page.clone());
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
        }
    });

    // 200 ms, not 1 s: a 1 s timer drifts against the deadline, so the shown
    // seconds would visibly stall or skip.
    let tick_timer = slint::Timer::default();
    tick_timer.start(slint::TimerMode::Repeated, Duration::from_millis(200), {
        let (ui, pomo, idle, page) = (ui.as_weak(), pomo.clone(), idle.clone(), page.clone());
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
            // Cheap, and catches midnight without a separate timer.
            if let Some(ui) = ui.upgrade() {
                let label = today_label();
                if ui.get_today_label() != label.as_str() {
                    ui.set_today_label(label.into());
                }
                while let Ok(msg) = google_msgs.try_recv() {
                    let mut c = cal.borrow_mut();
                    match msg {
                        Msg::Status(s) => {
                            // A new login may be a different account: drop its events.
                            if s == Status::WaitingForBrowser {
                                *c = CalState::default();
                            }
                            google_status = Some(s);
                        }
                        Msg::Synced { events, at } => {
                            if c.apply_sync(events, at) {
                                log_upcoming(&c);
                            }
                        }
                        Msg::SyncFailed(e) => c.apply_error(e),
                    }
                }
                push_calendar(&ui, &cal.borrow(), google_status.as_ref());
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

/// Calendar placeholder: the current / next event, and one status line (the login
/// state until logged in, then sync status). Runs every tick, so it only sets
/// properties that changed.
fn push_calendar(ui: &MainWindow, cal: &CalState, status: Option<&Status>) {
    let now = chrono::Local::now();
    let (next, line) = match status {
        Some(Status::LoggedIn { email }) => (cal.next_label(now), format!("{} · {email}", cal.sync_label())),
        Some(s) => (String::new(), s.label()),
        None => (String::new(), String::new()),
    };
    if ui.get_calendar_next() != next.as_str() {
        ui.set_calendar_next(next.into());
    }
    if ui.get_google_status() != line.as_str() {
        ui.set_google_status(line.into());
    }
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
