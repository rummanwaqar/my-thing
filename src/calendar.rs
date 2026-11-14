//! Calendar data as the UI sees it: events plus sync status. Doesn't know where
//! the events come from (`google::calendar` produces them). No UI code; every
//! method that depends on time takes `now`, so tests are deterministic.

use chrono::{DateTime, Local};

/// One occurrence (Google expands recurring events into one per instance).
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    /// Name of the calendar it's on.
    pub calendar: String,
    /// The calendar's color, e.g. "#9fe1e7".
    pub color: Option<String>,
    pub title: String,
    pub location: Option<String>,
    pub start: DateTime<Local>,
    /// Exclusive. For all-day events, the local midnight after the last day.
    pub end: DateTime<Local>,
    pub all_day: bool,
}

/// By start; all-day events first when they start at the same time; then by title.
pub fn sort(events: &mut [Event]) {
    events.sort_by(|a, b| (a.start, !a.all_day, &a.title).cmp(&(b.start, !b.all_day, &b.title)));
}

#[derive(Debug, Default)]
pub struct CalState {
    pub events: Vec<Event>,
    pub last_sync: Option<DateTime<Local>>,
    /// The latest sync failed (events are from the last good one).
    pub error: Option<String>,
}

impl CalState {
    /// Returns true if the events changed.
    pub fn apply_sync(&mut self, events: Vec<Event>, at: DateTime<Local>) -> bool {
        self.last_sync = Some(at);
        self.error = None;
        let changed = self.events != events;
        self.events = events;
        changed
    }

    /// Keeps the previous events.
    pub fn apply_error(&mut self, error: String) {
        self.error = Some(error);
    }

    /// The timed event in progress at `now`, else the next one to start.
    pub fn next(&self, now: DateTime<Local>) -> Option<&Event> {
        self.events.iter().find(|e| !e.all_day && e.end > now)
    }

    /// "Now: Standup (until 10:15)", "Next: 14:00 Lunch", "Next: Tue 09:00 Gym".
    /// Empty before the first sync.
    pub fn next_label(&self, now: DateTime<Local>) -> String {
        if self.last_sync.is_none() {
            return String::new();
        }
        let Some(e) = self.next(now) else {
            return "No upcoming events".into();
        };
        if e.start <= now {
            format!("Now: {} (until {})", e.title, e.end.format("%H:%M"))
        } else if e.start.date_naive() == now.date_naive() {
            format!("Next: {} {}", e.start.format("%H:%M"), e.title)
        } else {
            format!("Next: {} {}", e.start.format("%a %H:%M"), e.title)
        }
    }

    /// "12 events · synced 12:03", "Loading events…", "Sync failed (last synced 12:03): …".
    pub fn sync_label(&self) -> String {
        let synced = self.last_sync.map(|t| t.format("%H:%M").to_string());
        match (&self.error, synced) {
            (Some(e), Some(t)) => format!("Sync failed (last synced {t}): {e}"),
            (Some(e), None) => format!("Sync failed: {e}"),
            (None, None) => "Loading events…".into(),
            (None, Some(t)) => {
                let n = self.events.len();
                format!("{n} event{} · synced {t}", if n == 1 { "" } else { "s" })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, TimeZone};

    fn at(day: u32, h: u32, m: u32) -> DateTime<Local> {
        Local.with_ymd_and_hms(2026, 11, day, h, m, 0).unwrap()
    }

    fn ev(title: &str, start: DateTime<Local>, minutes: i64) -> Event {
        Event {
            calendar: "Work".into(),
            color: None,
            title: title.into(),
            location: None,
            start,
            end: start + Duration::minutes(minutes),
            all_day: false,
        }
    }

    fn all_day(title: &str, day: u32) -> Event {
        Event { all_day: true, ..ev(title, at(day, 0, 0), 24 * 60) }
    }

    fn synced(events: Vec<Event>) -> CalState {
        let mut s = CalState::default();
        s.apply_sync(events, at(14, 12, 3));
        s
    }

    #[test]
    fn sort_puts_all_day_first() {
        let mut v = vec![ev("B", at(14, 0, 0), 30), all_day("A", 14), ev("A", at(14, 0, 0), 30)];
        sort(&mut v);
        let titles: Vec<_> = v.iter().map(|e| (e.title.as_str(), e.all_day)).collect();
        assert_eq!(titles, [("A", true), ("A", false), ("B", false)]);
    }

    #[test]
    fn next_label_now_next_and_later_days() {
        let s = synced(vec![
            all_day("Holiday", 14),
            ev("Standup", at(14, 10, 0), 15),
            ev("Lunch", at(14, 14, 0), 60),
            ev("Gym", at(17, 9, 0), 60),
        ]);
        assert_eq!(s.next_label(at(14, 9, 0)), "Next: 10:00 Standup");
        assert_eq!(s.next_label(at(14, 10, 5)), "Now: Standup (until 10:15)");
        assert_eq!(s.next_label(at(14, 10, 15)), "Next: 14:00 Lunch");
        assert_eq!(s.next_label(at(14, 16, 0)), "Next: Tue 09:00 Gym");
        assert_eq!(s.next_label(at(18, 0, 0)), "No upcoming events");
        assert_eq!(CalState::default().next_label(at(14, 9, 0)), "");
    }

    #[test]
    fn sync_label_and_stale_data() {
        assert_eq!(CalState::default().sync_label(), "Loading events…");
        let mut s = synced(vec![ev("A", at(14, 9, 0), 30)]);
        assert_eq!(s.sync_label(), "1 event · synced 12:03");
        s.apply_error("offline".into());
        assert_eq!(s.sync_label(), "Sync failed (last synced 12:03): offline");
        assert_eq!(s.events.len(), 1);
    }

    #[test]
    fn apply_sync_reports_changes() {
        let mut s = synced(vec![ev("A", at(14, 9, 0), 30)]);
        assert!(!s.apply_sync(vec![ev("A", at(14, 9, 0), 30)], at(14, 12, 8)));
        assert!(s.apply_sync(vec![], at(14, 12, 13)));
    }
}
