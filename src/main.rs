use std::{cell::Cell, rc::Rc};

slint::include_modules!();

fn main() -> Result<(), slint::PlatformError> {
    let frames = Rc::new(Cell::new(0u32));

    let ui = MainWindow::new()?;
    ui.set_time(now_string());

    let frames_in_notifier = frames.clone();
    ui.window()
        .set_rendering_notifier(move |state, _graphics_api| {
            if let slint::RenderingState::AfterRendering = state {
                frames_in_notifier.set(frames_in_notifier.get() + 1);
            }
        })
        .expect("rendering notifier is not support in this renderer");

    let timer = slint::Timer::default();
    let ui_handle = ui.as_weak();
    let mut last = std::time::Instant::now();
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(1), move || {
        let elapsed = last.elapsed().as_secs_f32();
        last = std::time::Instant::now();
        let fps = frames.get() as f32 / elapsed;
        frames.set(0);
        if let Some(ui) = ui_handle.upgrade() {
            ui.set_fps(fps.round() as i32);
        }
    });

    let clock_timer = slint::Timer::default();
    let ui_handle = ui.as_weak();
    clock_timer.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(1), move || {
        if let Some(ui) = ui_handle.upgrade() {
            ui.set_time(now_string());
        }
    });

    ui.run()
}

fn now_string() -> slint::SharedString {
    chrono::Local::now().format("%H:%M:%S").to_string().into()
}