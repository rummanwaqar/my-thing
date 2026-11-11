slint::include_modules!();

// FPS: run with SLINT_DEBUG_PERFORMANCE=refresh_lazy,overlay (or console).
// Don't use Window::set_rendering_notifier: in Slint 1.18.1 it makes the
// linuxkms femtovg renderer resize its canvas to the window size after
// rotation (i-slint-renderer-femtovg lib.rs:255), shifting the whole image.

fn main() -> Result<(), slint::PlatformError> {
    let ui = MainWindow::new()?;
    ui.set_time(now_string());

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
