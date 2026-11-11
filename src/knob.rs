use evdev::{EventSummary, RelativeAxisCode};

/// Reads the Car Thing's rotary knob and forwards each step to the UI.
pub fn spawn(ui: slint::Weak<crate::MainWindow>) {
    std::thread::spawn(move || {
        let Some((path, mut dev)) =
            evdev::enumerate().find(|(_, d)| d.name() == Some("rotary"))
        else {
            eprintln!("knob: no 'rotary' input device found");
            return;
        };
        eprintln!("knob: reading {}", path.display());

        loop {
            let events = match dev.fetch_events() {
                Ok(events) => events,
                Err(e) => {
                    eprintln!("knob: read failed: {e}");
                    return;
                }
            };
            for ev in events {
                if let EventSummary::RelativeAxis(_, RelativeAxisCode::REL_HWHEEL, value) =
                    ev.destructure()
                {
                    let _ = ui.upgrade_in_event_loop(move |ui| ui.invoke_knob_turned(value));
                }
            }
        }
    });
}