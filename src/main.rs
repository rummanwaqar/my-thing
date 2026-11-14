mod backlight;
mod idle;
mod pages;
mod pomodoro;

#[cfg(feature = "device")]
mod knob;

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use idle::Idle;
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

    let labels: Vec<SharedString> = PRESETS
        .iter()
        .map(|p| format!("{} / {}", p.focus_min, p.break_min).into())
        .collect();
    ui.set_preset_labels(ModelRc::new(VecModel::from(labels)));

    // Each callback holds a Weak handle: a strong one would be a reference cycle
    // (the window owns the callback, the callback would own the window).
    // Every input first wakes the screen, then goes to the visible page only.
    // The Calendar ignores input for now.
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
                Page::Calendar => {}
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
