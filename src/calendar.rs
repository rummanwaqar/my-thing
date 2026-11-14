//! Calendar data as the UI sees it: events plus sync status, and the board
//! (what the Calendar page shows, every label formatted). Doesn't know where the
//! events come from (`google::calendar` produces them). No UI code; every
//! method that depends on time takes `now`, so tests are deterministic.

use chrono::{DateTime, Days, Local};

/// What kind of event it is, shown as a lettered badge (like a train line).
/// Decided by the data source (`google::calendar::kind`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    /// Focus time.
    Focus,
    /// I booked it (I'm the organizer).
    Booked,
    /// An invitation I haven't accepted.
    Pending,
    /// Everything else.
    #[default]
    Other,
}

impl Kind {
    /// The header legend's order.
    pub const ALL: [Kind; 4] = [Kind::Pending, Kind::Booked, Kind::Focus, Kind::Other];

    pub fn letter(self) -> char {
        match self {
            Kind::Focus => 'F',
            Kind::Booked => 'T',
            Kind::Pending => 'W',
            Kind::Other => 'P',
        }
    }

    /// From the design's badges.
    pub fn color(self) -> &'static str {
        match self {
            Kind::Focus => "#8e2c88",
            Kind::Booked => "#ff6319",
            Kind::Pending => "#0039a6",
            Kind::Other => "#007a33",
        }
    }
}

/// One occurrence (Google expands recurring events into one per instance).
#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    /// Name of the calendar it's on.
    pub calendar: String,
    pub kind: Kind,
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

// ---------- Board: what the screen shows ----------
//
// Layout (design "A · Departures list"): the event in progress as a NOW row,
// then the next departures as rows. The knob moves the first departure shown
// through the upcoming events.

/// One event, with every label already formatted for `now`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct EventView {
    pub title: String,
    /// "10:45 · Studio 4A": start ("Tue 10:45" when not today), then the location if any.
    pub detail: String,
    pub location: String,
    /// "10:15 – 10:32".
    pub range: String,
    /// Big countdown to the start: ("23", "MIN"), ("1:08", "HR"), ("2", "DAYS").
    pub big: (String, String),
    /// The kind's letter, for the round badge.
    pub badge: char,
    pub color: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct BoardView {
    /// The timed event in progress.
    pub now: Option<EventView>,
    /// Big time left: ("10", "MIN LEFT"), ("1:20", "HR LEFT").
    pub now_left: (String, String),
    /// How much of the event in progress is over, 0.0 → 1.0.
    pub now_progress: f32,
    /// Upcoming events from the knob's position, up to 4.
    pub upcoming: Vec<EventView>,
    /// Today's all-day events.
    pub all_day_count: usize,
    /// One badge per kind of event in the window (header legend).
    pub legend: Vec<EventView>,
}

/// Number of upcoming timed events (how far the knob can move).
pub fn upcoming_count(events: &[Event], now: DateTime<Local>) -> usize {
    upcoming(events, now).count()
}

fn upcoming(events: &[Event], now: DateTime<Local>) -> impl Iterator<Item = &Event> {
    events.iter().filter(move |e| !e.all_day && e.start > now)
}

/// The upcoming event the knob was turned to, remembered by identity rather
/// than position: a sync or an event starting would shift a position.
#[derive(Debug, Clone, PartialEq)]
pub struct Pick {
    start: DateTime<Local>,
    title: String,
}

/// The pick for the upcoming event at `offset` (None for offset 0: the actual next event).
pub fn pick_at(events: &[Event], now: DateTime<Local>, offset: usize) -> Option<Pick> {
    if offset == 0 {
        return None;
    }
    upcoming(events, now).nth(offset).map(|e| Pick { start: e.start, title: e.title.clone() })
}

/// Where `pick` is among the upcoming events now. If it has started or is gone,
/// the first event after its start time; 0 (the next event) without a pick.
pub fn offset_of(events: &[Event], now: DateTime<Local>, pick: Option<&Pick>) -> usize {
    let Some(p) = pick else { return 0 };
    let list: Vec<&Event> = upcoming(events, now).collect();
    list.iter()
        .position(|e| e.start == p.start && e.title == p.title)
        .or_else(|| list.iter().position(|e| e.start >= p.start))
        .unwrap_or(list.len().saturating_sub(1))
}

/// The board for `now`, with the departures starting `offset` events ahead.
pub fn board(events: &[Event], now: DateTime<Local>, offset: usize) -> BoardView {
    let current = events.iter().find(|e| !e.all_day && e.start <= now && now < e.end);
    let upcoming: Vec<&Event> = upcoming(events, now).collect();
    let offset = offset.min(upcoming.len().saturating_sub(1));
    let after = upcoming.get(offset..).unwrap_or_default();
    let today = now.date_naive();
    BoardView {
        now: current.map(|e| event_view(e, now)),
        now_left: current.map(|e| time_left(e.end - now)).unwrap_or_default(),
        now_progress: current.map_or(0.0, |e| {
            let total = (e.end - e.start).num_seconds().max(1) as f32;
            ((now - e.start).num_seconds() as f32 / total).clamp(0.0, 1.0)
        }),
        upcoming: after.iter().take(4).map(|e| event_view(e, now)).collect(),
        all_day_count: events
            .iter()
            .filter(|e| e.all_day && e.start.date_naive() <= today && today < e.end.date_naive())
            .count(),
        legend: Kind::ALL
            .into_iter()
            .filter(|k| events.iter().any(|e| e.kind == *k))
            .map(|k| EventView { badge: k.letter(), color: Some(k.color().into()), ..Default::default() })
            .collect(),
    }
}

fn event_view(e: &Event, now: DateTime<Local>) -> EventView {
    let when = if e.start.date_naive() == now.date_naive() {
        e.start.format("%H:%M").to_string()
    } else {
        e.start.format("%a %H:%M").to_string()
    };
    EventView {
        title: e.title.clone(),
        detail: match &e.location {
            Some(l) => format!("{when} · {l}"),
            None => when,
        },
        location: e.location.clone().unwrap_or_default(),
        range: format!("{} – {}", e.start.format("%H:%M"), e.end.format("%H:%M")),
        big: big_countdown(e.start - now),
        badge: e.kind.letter(),
        color: Some(e.kind.color().into()),
    }
}

/// Whole minutes, rounded up: 30 s away is "1 MIN", not "0 MIN".
fn minutes_until(d: chrono::Duration) -> i64 {
    (d.num_seconds().max(0) + 59) / 60
}

/// A departure's big number and unit.
fn big_countdown(d: chrono::Duration) -> (String, String) {
    let m = minutes_until(d);
    match m {
        0..60 => (m.to_string(), "MIN".into()),
        60..1440 => (format!("{}:{:02}", m / 60, m % 60), "HR".into()),
        _ => {
            let days = m / 1440;
            (days.to_string(), if days == 1 { "DAY" } else { "DAYS" }.into())
        }
    }
}

/// The event in progress: ("10", "MIN LEFT"), ("1:20", "HR LEFT").
fn time_left(d: chrono::Duration) -> (String, String) {
    let m = minutes_until(d);
    if m < 60 {
        (m.to_string(), "MIN LEFT".into())
    } else {
        (format!("{}:{:02}", m / 60, m % 60), "HR LEFT".into())
    }
}

// ---------- Reminders: the takeover before an event ----------
//
// Design "D · Reminder takeover". A timed event's reminder is due from
// `remind_before()` before its start until `keep_after_start()` after it (or
// its end, if sooner), unless dismissed.

/// How long before an event its reminder takes over the screen.
pub fn remind_before() -> chrono::Duration {
    chrono::Duration::minutes(5)
}

/// How long after the start an undismissed reminder stays up (then it dismisses itself).
fn keep_after_start() -> chrono::Duration {
    chrono::Duration::minutes(5)
}

/// Which reminders have been dismissed. Events are matched by start and title
/// (like [`Pick`]), so a dismissal survives syncs.
#[derive(Debug, Default)]
pub struct Reminders {
    dismissed: Vec<Pick>,
}

impl Pick {
    fn of(e: &Event) -> Pick {
        Pick { start: e.start, title: e.title.clone() }
    }

    fn matches(&self, e: &Event) -> bool {
        self.start == e.start && self.title == e.title
    }
}

impl Reminders {
    /// The reminder to show at `now`: the earliest event that's due and not dismissed.
    pub fn due<'a>(&self, events: &'a [Event], now: DateTime<Local>) -> Option<&'a Event> {
        events.iter().find(|e| {
            !e.all_day
                && e.start - remind_before() <= now
                && now < (e.start + keep_after_start()).min(e.end.max(e.start + chrono::Duration::seconds(1)))
                && !self.dismissed.iter().any(|p| p.matches(e))
        })
    }

    pub fn dismiss(&mut self, e: &Event) {
        // Forget dismissals of events long gone.
        let cutoff = e.start - chrono::Duration::days(1);
        self.dismissed.retain(|p| !p.matches(e) && p.start > cutoff);
        self.dismissed.push(Pick::of(e));
    }
}

/// The takeover's labels: detail "10:45 – 11:30 · Studio 4A", and the big
/// countdown to the start ("2", "MIN"), or since it ("3", "MIN AGO").
pub fn reminder_view(e: &Event, now: DateTime<Local>) -> EventView {
    let v = event_view(e, now);
    EventView {
        detail: match &e.location {
            Some(l) => format!("{} · {l}", v.range),
            None => v.range.clone(),
        },
        big: if now < e.start {
            big_countdown(e.start - now)
        } else {
            ((now - e.start).num_minutes().to_string(), "MIN AGO".into())
        },
        ..v
    }
}

/// Made-up events around `now`, for `--demo`: the Calendar page can be worked
/// on without a Google login.
pub fn sample_events(now: DateTime<Local>) -> Vec<Event> {
    use chrono::{Duration, NaiveTime, TimeZone};
    let day = |offset: u64| now.date_naive() + Days::new(offset);
    let at = |offset: u64, h: u32, m: u32| {
        Local
            .from_local_datetime(&day(offset).and_time(NaiveTime::from_hms_opt(h, m, 0).unwrap()))
            .earliest()
            .unwrap_or(now)
    };
    let ev = |kind: Kind, title: &str, start: DateTime<Local>, minutes: i64, location: &str| Event {
        calendar: "Demo".into(),
        kind,
        title: title.into(),
        location: (!location.is_empty()).then(|| location.into()),
        start,
        end: start + Duration::minutes(minutes),
        all_day: false,
    };
    let mins = Duration::minutes;
    let mut v = vec![
        ev(Kind::Other, "Standup", now - mins(90), 15, "Google Meet"),
        ev(Kind::Booked, "Team coffee", now - mins(20), 30, ""),
        // Within 5 minutes, so --demo opens on the reminder takeover (Back dismisses it).
        ev(Kind::Pending, "Launch readiness review", now + mins(3), 45, "Studio 4A · Video call"),
        ev(Kind::Booked, "1:1 sync", now + mins(48), 20, ""),
        ev(Kind::Focus, "Deep work", now + mins(68), 60, ""),
        ev(Kind::Other, "Lunch", now + mins(128), 60, "Cafe Nero"),
        ev(Kind::Pending, "Design sync", now + mins(220), 30, "Room 2.1"),
        ev(Kind::Booked, "Gym", now + mins(430), 60, ""),
        ev(Kind::Other, "Planning", at(1, 10, 0), 90, "Room 2.1"),
    ];
    v.push(Event { all_day: true, start: at(0, 0, 0), end: at(1, 0, 0), ..ev(Kind::Other, "Offsite", now, 0, "Lisbon") });
    sort(&mut v);
    v
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
            kind: Kind::Other,
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

    // ---------- Board ----------

    fn titled(v: &[EventView]) -> Vec<&str> {
        v.iter().map(|e| e.title.as_str()).collect()
    }

    fn day_events() -> Vec<Event> {
        vec![
            all_day("Offsite", 14),
            ev("Coffee", at(14, 10, 0), 30),
            ev("Launch review", at(14, 10, 45), 30),
            ev("1:1 sync", at(14, 11, 10), 20),
            ev("Deep work", at(14, 11, 30), 60),
            ev("Lunch", at(14, 12, 30), 60),
            ev("Design sync", at(14, 14, 0), 30),
            ev("Gym", at(14, 17, 30), 60),
            ev("Planning", at(16, 9, 0), 60),
        ]
    }

    #[test]
    fn board_now_row_and_departures() {
        let b = board(&day_events(), at(14, 10, 22), 0);
        let now = b.now.as_ref().unwrap();
        assert_eq!(now.title, "Coffee");
        assert_eq!(now.range, "10:00 – 10:30");
        assert_eq!(b.now_left, ("8".to_string(), "MIN LEFT".to_string()));
        assert!((b.now_progress - 22.0 / 30.0).abs() < 0.01);
        assert_eq!(titled(&b.upcoming), ["Launch review", "1:1 sync", "Deep work", "Lunch"]);
        assert_eq!(b.upcoming[0].big, ("23".to_string(), "MIN".to_string()));
        assert_eq!(b.upcoming[0].detail, "10:45");
        assert_eq!(b.upcoming[2].big, ("1:08".to_string(), "HR".to_string()));
        assert_eq!(b.upcoming[0].badge, 'P'); // Kind::Other
        assert_eq!(b.all_day_count, 1);
        assert_eq!(b.legend.len(), 1); // all test events are Kind::Other
        assert_eq!(b.legend[0].badge, 'P');
        let with_place = Event { location: Some("Room 1".into()), ..ev("Standup", at(14, 11, 0), 15) };
        assert_eq!(event_view(&with_place, at(14, 10, 22)).detail, "11:00 · Room 1"); // time first
    }

    #[test]
    fn knob_offset_moves_the_departures_and_clamps() {
        let events = day_events();
        let b = board(&events, at(14, 10, 22), 2);
        assert_eq!(titled(&b.upcoming), ["Deep work", "Lunch", "Design sync", "Gym"]);
        assert_eq!(b.now.unwrap().title, "Coffee"); // NOW stays
        let last = board(&events, at(14, 10, 22), 99);
        assert_eq!(titled(&last.upcoming), ["Planning"]);
        assert_eq!(last.upcoming[0].detail, "Mon 09:00");
        assert_eq!(last.upcoming[0].big, ("1".to_string(), "DAY".to_string()));
        assert_eq!(upcoming_count(&events, at(14, 10, 22)), 7);
    }

    #[test]
    fn pick_survives_syncs_and_events_starting() {
        let events = day_events();
        let first = |b: BoardView| b.upcoming[0].title.clone();
        let pick = pick_at(&events, at(14, 10, 22), 2); // Deep work
        assert_eq!(first(board(&events, at(14, 10, 22), offset_of(&events, at(14, 10, 22), pick.as_ref()))), "Deep work");
        // Launch review starts: Deep work moves up one place but stays picked.
        let later = at(14, 10, 50);
        assert_eq!(offset_of(&events, later, pick.as_ref()), 1);
        assert_eq!(first(board(&events, later, 1)), "Deep work");
        // A sync drops it: the next event after its start time.
        let without: Vec<Event> = events.iter().filter(|e| e.title != "Deep work").cloned().collect();
        assert_eq!(first(board(&without, later, offset_of(&without, later, pick.as_ref()))), "Lunch");
        // No pick: the actual next event.
        assert_eq!(pick_at(&events, later, 0), None);
        assert_eq!(offset_of(&events, later, None), 0);
    }

    #[test]
    fn free_and_empty_boards() {
        let b = board(&day_events(), at(14, 10, 40), 0); // between Coffee and Launch review
        assert!(b.now.is_none());
        assert_eq!(b.upcoming[0].big, ("5".to_string(), "MIN".to_string()));
        let empty = board(&[], at(14, 10, 0), 0);
        assert!(empty.now.is_none() && empty.upcoming.is_empty() && empty.legend.is_empty());
        assert_eq!(empty.all_day_count, 0);
    }

    #[test]
    fn countdowns_round_up() {
        assert_eq!(time_left(chrono::Duration::seconds(30)), ("1".to_string(), "MIN LEFT".to_string()));
        assert_eq!(time_left(chrono::Duration::minutes(80)), ("1:20".to_string(), "HR LEFT".to_string()));
        assert_eq!(big_countdown(chrono::Duration::minutes(3 * 1440)), ("3".to_string(), "DAYS".to_string()));
    }

    // ---------- Reminders ----------

    #[test]
    fn reminder_window() {
        let events = day_events(); // Launch review 10:45–11:15, 1:1 sync 11:10–11:30
        let r = Reminders::default();
        let due = |now| r.due(&events, now).map(|e| e.title.as_str());
        assert_eq!(due(at(14, 10, 39)), None);
        assert_eq!(due(at(14, 10, 40)), Some("Launch review")); // 5 min before
        assert_eq!(due(at(14, 10, 49)), Some("Launch review")); // still up after the start
        assert_eq!(due(at(14, 10, 50)), None); // 5 min after: dismissed by itself
        assert_eq!(due(at(14, 11, 5)), Some("1:1 sync"));
        // Short event: gone when it ends.
        let short = vec![ev("Quick", at(14, 9, 0), 3)];
        assert!(r.due(&short, at(14, 9, 2)).is_some());
        assert!(r.due(&short, at(14, 9, 3)).is_none());
        // All-day events never take over.
        assert!(r.due(&[all_day("Offsite", 14)], at(14, 0, 0)).is_none());
    }

    #[test]
    fn dismiss_shows_the_next_and_survives_syncs() {
        let events = vec![ev("A", at(14, 10, 0), 30), ev("B", at(14, 10, 2), 30)];
        let mut r = Reminders::default();
        let now = at(14, 9, 58); // both within 5 minutes
        assert_eq!(r.due(&events, now).unwrap().title, "A");
        r.dismiss(&events[0]);
        assert_eq!(r.due(&events, now).unwrap().title, "B"); // the next one due
        r.dismiss(&events[1]);
        assert!(r.due(&events, now).is_none());
        // A sync brings new Event values with the same start and title: still dismissed.
        let synced: Vec<Event> = events.clone();
        assert!(r.due(&synced, at(14, 10, 1)).is_none());
    }

    #[test]
    fn reminder_labels() {
        let e = Event { location: Some("Studio 4A".into()), ..ev("Launch review", at(14, 10, 45), 45) };
        let v = reminder_view(&e, at(14, 10, 43));
        assert_eq!(v.detail, "10:45 – 11:30 · Studio 4A");
        assert_eq!(v.big, ("2".to_string(), "MIN".to_string()));
        assert_eq!(reminder_view(&e, at(14, 10, 48)).big, ("3".to_string(), "MIN AGO".to_string()));
        assert_eq!(reminder_view(&ev("X", at(14, 10, 45), 15), at(14, 10, 43)).detail, "10:45 – 11:00");
    }
}
