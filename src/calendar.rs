//! Calendar data as the UI sees it: events plus sync status, and the agenda
//! (the scrollable list of day headers and events). Doesn't know where the
//! events come from (`google::calendar` produces them). No UI code; every
//! method that depends on time takes `now`, so tests are deterministic.

use chrono::{DateTime, Days, Local, NaiveDate};

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

// ---------- Agenda: the list on screen ----------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    /// "Today · Sat 14 Nov"
    Day,
    Event,
    /// "Nothing scheduled" under Today when it has no events.
    Empty,
}

/// One line of the agenda.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub kind: RowKind,
    /// Day header text, or the event's time ("09:00–09:30", "All day", "until 01:00").
    pub label: String,
    pub title: String,
    /// Location, if any.
    pub detail: String,
    pub color: Option<String>,
    /// In progress (timed events only).
    pub now: bool,
    /// Already over.
    pub past: bool,
}

/// Day headers and events from today on. All-day events appear on every day
/// they cover; anything that started before today is listed under Today.
pub fn rows(events: &[Event], now: DateTime<Local>) -> Vec<Row> {
    let today = now.date_naive();
    let mut entries: Vec<(NaiveDate, &Event)> = Vec::new();
    for e in events {
        let first = e.start.date_naive().max(today);
        if e.all_day {
            let mut d = first;
            while d < e.end.date_naive() {
                entries.push((d, e));
                d = d + Days::new(1);
            }
        } else {
            entries.push((first, e));
        }
    }
    // Stable sort: events keep their start order within a day; all-day first.
    entries.sort_by_key(|(d, e)| (*d, !e.all_day));

    let mut out = Vec::new();
    if entries.first().is_none_or(|(d, _)| *d != today) {
        out.push(day_row(today, today));
        out.push(Row {
            kind: RowKind::Empty,
            label: String::new(),
            title: "Nothing scheduled".into(),
            detail: String::new(),
            color: None,
            now: false,
            past: false,
        });
    }
    let mut current: Option<NaiveDate> = None;
    for (d, e) in entries {
        if current != Some(d) {
            out.push(day_row(d, today));
            current = Some(d);
        }
        out.push(Row {
            kind: RowKind::Event,
            label: time_label(e, d),
            title: e.title.clone(),
            detail: e.location.clone().unwrap_or_default(),
            color: e.color.clone(),
            now: !e.all_day && e.start <= now && now < e.end,
            past: e.end <= now,
        });
    }
    out
}

fn day_row(d: NaiveDate, today: NaiveDate) -> Row {
    let date = d.format("%a %-d %b");
    let label = if d == today {
        format!("Today · {date}")
    } else if Some(d) == today.succ_opt() {
        format!("Tomorrow · {date}")
    } else {
        date.to_string()
    };
    Row {
        kind: RowKind::Day,
        label,
        title: String::new(),
        detail: String::new(),
        color: None,
        now: false,
        past: false,
    }
}

/// The time column for `e` when listed under day `d`.
fn time_label(e: &Event, d: NaiveDate) -> String {
    let (start_day, end_day) = (e.start.date_naive(), e.end.date_naive());
    let hm = |t: &DateTime<Local>| t.format("%H:%M").to_string();
    if e.all_day || (start_day < d && end_day > d) {
        "All day".into()
    } else if start_day < d {
        format!("until {}", hm(&e.end))
    } else if e.end == e.start {
        hm(&e.start)
    } else {
        format!("{}–{}", hm(&e.start), hm(&e.end))
    }
}

/// Where "now" is in the list: the first event that isn't over, or its day
/// header if it's the first event of its day.
pub fn anchor(rows: &[Row]) -> usize {
    let Some(i) = rows.iter().position(|r| r.kind != RowKind::Day && !r.past) else {
        return 0;
    };
    if i > 0 && rows[i - 1].kind == RowKind::Day { i - 1 } else { i }
}

/// The rows plus the scroll position. Follows "now" until the knob scrolls it,
/// and again after [`Agenda::back_to_now`].
#[derive(Debug, Default)]
pub struct Agenda {
    pub rows: Vec<Row>,
    /// Index of the top visible row.
    pub first: usize,
    scrolled: bool,
}

impl Agenda {
    /// Rebuilds the rows (cheap; called every tick so "now" and past markers move).
    pub fn refresh(&mut self, events: &[Event], now: DateTime<Local>) {
        self.rows = rows(events, now);
        self.first = if self.scrolled { self.first.min(self.last()) } else { anchor(&self.rows) };
    }

    /// Knob turn: one row per detent, clamped to the list.
    pub fn scroll(&mut self, delta: i32) {
        self.scrolled = true;
        let target = self.first as i64 + i64::from(delta);
        self.first = target.clamp(0, self.last() as i64) as usize;
    }

    pub fn back_to_now(&mut self) {
        self.scrolled = false;
        self.first = anchor(&self.rows);
    }

    /// Up to `n` rows from the top visible one.
    pub fn visible(&self, n: usize) -> &[Row] {
        let start = self.first.min(self.rows.len());
        &self.rows[start..(start + n).min(self.rows.len())]
    }

    fn last(&self) -> usize {
        self.rows.len().saturating_sub(1)
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
    let ev = |cal: &str, color: &str, title: &str, start: DateTime<Local>, minutes: i64, location: &str| Event {
        calendar: cal.into(),
        color: Some(color.into()),
        title: title.into(),
        location: (!location.is_empty()).then(|| location.into()),
        start,
        end: start + Duration::minutes(minutes),
        all_day: false,
    };
    let (work, home) = (("Work", "#4986e7"), ("Personal", "#7bd148"));
    let mut v = vec![
        ev(work.0, work.1, "Standup", now - Duration::minutes(90), 15, "Google Meet"),
        ev(work.0, work.1, "Design review", now - Duration::minutes(20), 60, "Room 4.2"),
        ev(home.0, home.1, "Lunch with Sam", now + Duration::minutes(75), 60, "Cafe Nero"),
        ev(work.0, work.1, "1:1 with manager", now + Duration::minutes(180), 30, ""),
        ev(work.0, work.1, "Planning", at(1, 10, 0), 90, "Room 2.1"),
        ev(home.0, home.1, "Gym", at(1, 18, 30), 60, ""),
        ev(work.0, work.1, "Quarterly all-hands with a very long title that will not fit", at(2, 15, 0), 60, ""),
    ];
    v.push(Event { all_day: true, start: at(2, 0, 0), end: at(4, 0, 0), ..ev(home.0, "#f691b2", "Offsite", now, 0, "Lisbon") });
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

    // ---------- Agenda ----------

    fn labels(rows: &[Row]) -> Vec<String> {
        rows.iter()
            .map(|r| match r.kind {
                RowKind::Day => format!("# {}", r.label),
                _ => format!("{} {}", r.label, r.title).trim().to_string(),
            })
            .collect()
    }

    #[test]
    fn rows_group_by_day_with_multi_day_and_carried_over_events() {
        let late = ev("Late", at(13, 23, 0), 120); // yesterday 23:00 → today 01:00
        let offsite = Event { all_day: true, end: at(17, 0, 0), ..ev("Offsite", at(15, 0, 0), 0) };
        let events = vec![
            late,
            ev("Standup", at(14, 10, 0), 15),
            ev("Party", at(14, 23, 0), 120),
            offsite,
            ev("Gym", at(16, 9, 0), 60),
        ];
        assert_eq!(
            labels(&rows(&events, at(14, 9, 0))),
            [
                "# Today · Sat 14 Nov",
                "until 01:00 Late",
                "10:00–10:15 Standup",
                "23:00–01:00 Party",
                "# Tomorrow · Sun 15 Nov",
                "All day Offsite",
                "# Mon 16 Nov",
                "All day Offsite",
                "09:00–10:00 Gym",
            ]
        );
    }

    #[test]
    fn empty_today_still_has_a_header() {
        let events = vec![ev("Gym", at(16, 9, 0), 60)];
        assert_eq!(
            labels(&rows(&events, at(14, 9, 0))),
            ["# Today · Sat 14 Nov", "Nothing scheduled", "# Mon 16 Nov", "09:00–10:00 Gym"]
        );
    }

    #[test]
    fn now_and_past_flags_and_anchor() {
        let events = vec![
            ev("A", at(14, 8, 0), 30),
            ev("B", at(14, 10, 0), 60),
            ev("C", at(15, 9, 0), 60),
        ];
        let r = rows(&events, at(14, 10, 30));
        assert!(r[1].past && !r[1].now);
        assert!(r[2].now && !r[2].past);
        assert_eq!(anchor(&r), 2); // B: today's header would show past A first

        let r = rows(&events, at(14, 12, 0));
        assert_eq!(anchor(&r), 3); // tomorrow's header, then C

        let r = rows(&events, at(14, 7, 0));
        assert_eq!(anchor(&r), 0); // Today's header above A
    }

    #[test]
    fn agenda_follows_now_until_scrolled() {
        let events = vec![ev("A", at(14, 8, 0), 30), ev("B", at(14, 10, 0), 60), ev("C", at(15, 9, 0), 60)];
        let mut a = Agenda::default();
        a.refresh(&events, at(14, 7, 0));
        assert_eq!(a.first, 0);
        a.refresh(&events, at(14, 9, 0));
        assert_eq!(a.first, 2); // A is over: follows to B

        a.scroll(-5);
        assert_eq!(a.first, 0); // clamped
        a.refresh(&events, at(14, 12, 0));
        assert_eq!(a.first, 0); // stays where the user scrolled
        a.scroll(100);
        assert_eq!(a.first, a.rows.len() - 1);
        assert_eq!(a.visible(3).len(), 1);

        a.back_to_now();
        assert_eq!(a.first, 3);
    }
}
