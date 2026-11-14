//! Which app is on screen. The preset buttons (1–4) switch between them.
//!
//! Only the visible page receives knob / Enter / Esc input; every app keeps its
//! state and keeps running while hidden.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Calendar,
    Pomodoro,
}

impl Page {
    /// The page shown at startup.
    pub const FIRST: Page = Page::Calendar;

    /// Preset button → page. Buttons without an app return None (stay where we are).
    pub fn from_preset(n: i32) -> Option<Page> {
        match n {
            1 => Some(Page::Calendar),
            2 => Some(Page::Pomodoro),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_map_to_pages() {
        assert_eq!(Page::from_preset(1), Some(Page::Calendar));
        assert_eq!(Page::from_preset(2), Some(Page::Pomodoro));
    }

    #[test]
    fn unused_presets_do_nothing() {
        assert_eq!(Page::from_preset(3), None);
        assert_eq!(Page::from_preset(4), None);
        assert_eq!(Page::from_preset(0), None);
    }
}
