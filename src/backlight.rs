//! Screen backlight: full brightness or dimmed.
//!
//! On the device this writes the panel's sysfs backlight. On the desktop there is
//! no backlight, so it only logs, which is enough to follow the logic in `cargo run`.

/// Normal brightness (max_brightness is 160).
#[cfg(feature = "device")]
const BRIGHT_LEVEL: u32 = 100;

/// Brightness when dimmed. superbird-als uses 16 as its floor; much lower can
/// switch the LP8556 panel fully off. Trying 5.
#[cfg(feature = "device")]
const DIM_LEVEL: u32 = 5;

#[cfg(feature = "device")]
const BRIGHTNESS: &str = "/sys/class/backlight/backlight/brightness";

pub fn set_dimmed(dimmed: bool) {
    eprintln!("backlight: {}", if dimmed { "dim" } else { "bright" });

    #[cfg(feature = "device")]
    {
        let level = if dimmed { DIM_LEVEL } else { BRIGHT_LEVEL };
        if let Err(e) = std::fs::write(BRIGHTNESS, level.to_string()) {
            eprintln!("backlight: write {level} to {BRIGHTNESS} failed: {e}");
        }
    }
}
