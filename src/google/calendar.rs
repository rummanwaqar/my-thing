//! Google Calendar API v3 → [`crate::calendar::Event`]s.
//!
//! `calendarList.list` gives the calendars ticked in Google Calendar; for each,
//! `events.list` with `singleEvents=true` returns every instance in the window
//! with recurrence, moved and cancelled instances already applied by Google.

use chrono::{DateTime, Local, NaiveDate, NaiveTime, SecondsFormat};
use serde::Deserialize;

use super::auth::{Auth, AuthError};
use super::store::StoredLogin;
use crate::calendar::{self, Event, Kind};

/// Guard against a server that keeps sending page tokens.
const MAX_PAGES: usize = 20;

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEntry {
    pub id: String,
    #[serde(default)]
    pub summary: String,
    pub summary_override: Option<String>,
    #[serde(default)]
    pub selected: bool,
    #[serde(default)]
    pub hidden: bool,
}

impl CalendarEntry {
    fn name(&self) -> &str {
        self.summary_override.as_deref().unwrap_or(&self.summary)
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page<T> {
    #[serde(default = "Vec::new")]
    items: Vec<T>,
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiEvent {
    #[serde(default)]
    status: String,
    summary: Option<String>,
    location: Option<String>,
    start: Option<When>,
    end: Option<When>,
    #[serde(default)]
    attendees: Vec<Attendee>,
    organizer: Option<Organizer>,
    #[serde(default)]
    event_type: String,
}

#[derive(Debug, Deserialize)]
struct Organizer {
    #[serde(rename = "self", default)]
    is_self: bool,
}

/// Either `dateTime` (RFC 3339, timed) or `date` (YYYY-MM-DD, all-day).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct When {
    date_time: Option<DateTime<chrono::FixedOffset>>,
    date: Option<NaiveDate>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Attendee {
    #[serde(rename = "self", default)]
    is_self: bool,
    #[serde(default)]
    response_status: String,
}

/// The calendars to show: ticked in Google Calendar and not hidden.
fn shown(entries: Vec<CalendarEntry>) -> Vec<CalendarEntry> {
    entries.into_iter().filter(|c| c.selected && !c.hidden).collect()
}

/// The badge rules, first match wins:
/// F focus time · T I booked it (I'm the organizer) · W an invitation I haven't
/// accepted (needs action or tentative) · P everything else.
fn kind(e: &ApiEvent) -> Kind {
    if e.event_type == "focusTime" {
        Kind::Focus
    } else if e.organizer.as_ref().is_some_and(|o| o.is_self) {
        Kind::Booked
    } else if e.attendees.iter().any(|a| a.is_self && a.response_status != "accepted") {
        Kind::Pending
    } else {
        Kind::Other
    }
}

/// Location rule for office rooms. Room resources at 285W 5th look like
/// "285W 5th-4FL-Room 306 (6)": building, floor, room, and occupancy in
/// brackets. Keep just the room ("Room 306", "Main Conference Room (311)").
/// Other locations are left alone. A list of rooms ("a, b") is tidied per room.
fn tidy_location(location: &str) -> String {
    location
        .split(',')
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .map(tidy_room)
        .collect::<Vec<_>>()
        .join(", ")
}

fn tidy_room(part: &str) -> &str {
    let squashed: String = part.chars().filter(|c| !c.is_whitespace()).collect::<String>().to_ascii_lowercase();
    if !squashed.contains("285w5th") {
        return part;
    }
    let Some(room) = after_floor(part) else {
        return part; // not a room resource: no floor marker
    };
    // Occupancy is the last bracket group; an earlier one (a room number) stays.
    match room.strip_suffix(')').and_then(|r| r.rfind('(')) {
        Some(open) => room[..open].trim_end(),
        None => room,
    }
}

/// What follows the last floor marker: digits, "FL", then "-" (any case,
/// spaces allowed before and after the dash).
fn after_floor(s: &str) -> Option<&str> {
    // ASCII lowercasing keeps byte offsets, so indices map back onto `s`.
    let b = s.to_ascii_lowercase().into_bytes();
    let mut found = None;
    for i in 1..b.len().saturating_sub(1) {
        if b[i] == b'f' && b[i + 1] == b'l' && b[i - 1].is_ascii_digit() {
            let mut j = i + 2;
            while j < b.len() && b[j] == b' ' {
                j += 1;
            }
            if j < b.len() && b[j] == b'-' {
                found = Some(j + 1);
            }
        }
    }
    found.map(|k| s[k..].trim()).filter(|room| !room.is_empty())
}

/// None for events that shouldn't appear: cancelled, declined by me, Workspace
/// "working location" markers, or without usable times.
fn to_event(e: ApiEvent, cal: &CalendarEntry) -> Option<Event> {
    if e.status == "cancelled" || e.event_type == "workingLocation" {
        return None;
    }
    if e.attendees.iter().any(|a| a.is_self && a.response_status == "declined") {
        return None;
    }
    let (start, all_day) = local(e.start.as_ref()?)?;
    let (end, _) = local(e.end.as_ref()?)?;
    Some(Event {
        calendar: cal.name().to_string(),
        kind: kind(&e),
        title: e
            .summary
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "(No title)".into()),
        location: e.location.map(|l| tidy_location(&l)).filter(|l| !l.is_empty()),
        start,
        end: end.max(start),
        all_day,
    })
}

/// (instant, is all-day). All-day dates are local midnights.
fn local(w: &When) -> Option<(DateTime<Local>, bool)> {
    match (w.date_time, w.date) {
        (Some(dt), _) => Some((dt.with_timezone(&Local), false)),
        (None, Some(d)) => Some((d.and_time(NaiveTime::MIN).and_local_timezone(Local).earliest()?, true)),
        (None, None) => None,
    }
}

/// Every event in `[from, to)` on every shown calendar, sorted.
pub fn fetch(
    auth: &Auth,
    login: &mut StoredLogin,
    from: DateTime<Local>,
    to: DateTime<Local>,
) -> Result<Vec<Event>, AuthError> {
    let fields = "items(id,summary,summaryOverride,selected,hidden),nextPageToken";
    let calendars = shown(paged(auth, login, &["users", "me", "calendarList"], &[("fields", fields)])?);

    let time_min = from.to_rfc3339_opts(SecondsFormat::Secs, false);
    let time_max = to.to_rfc3339_opts(SecondsFormat::Secs, false);
    let fields = "items(id,status,summary,location,start,end,attendees(self,responseStatus),organizer(self),eventType),nextPageToken";
    let query = [
        ("timeMin", time_min.as_str()),
        ("timeMax", time_max.as_str()),
        ("singleEvents", "true"),
        ("orderBy", "startTime"),
        ("maxResults", "250"),
        ("fields", fields),
    ];

    let mut events = Vec::new();
    for cal in &calendars {
        let items: Vec<ApiEvent> = paged(auth, login, &["calendars", &cal.id, "events"], &query)?;
        events.extend(items.into_iter().filter_map(|e| to_event(e, cal)));
    }
    calendar::sort(&mut events);
    Ok(events)
}

/// GETs every page of a list endpoint.
fn paged<T: serde::de::DeserializeOwned>(
    auth: &Auth,
    login: &mut StoredLogin,
    segments: &[&str],
    query: &[(&str, &str)],
) -> Result<Vec<T>, AuthError> {
    let mut items = Vec::new();
    let mut token: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut q = query.to_vec();
        if let Some(t) = &token {
            q.push(("pageToken", t));
        }
        let page: Page<T> = auth.get_json(login, &auth.api_url(segments, &q))?;
        items.extend(page.items);
        match page.next_page_token {
            Some(t) => token = Some(t),
            None => return Ok(items),
        }
    }
    Err(AuthError::Other(format!("more than {MAX_PAGES} pages from {}", segments.join("/"))))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google::testutil::{auth_against, json_response, serve};
    use chrono::{TimeZone, Utc};

    fn cal() -> CalendarEntry {
        CalendarEntry {
            id: "me@example.com".into(),
            summary: "me@example.com".into(),
            summary_override: Some("Work".into()),
            selected: true,
            hidden: false,
        }
    }

    fn event(json: &str) -> Option<Event> {
        to_event(serde_json::from_str(json).unwrap(), &cal())
    }

    #[test]
    fn timed_event_with_offset() {
        let e = event(
            r#"{"status":"confirmed","summary":"Standup","location":"Room 1",
                "start":{"dateTime":"2026-11-16T10:00:00+01:00","timeZone":"Europe/Berlin"},
                "end":{"dateTime":"2026-11-16T10:15:00+01:00"}}"#,
        )
        .unwrap();
        assert_eq!(e.title, "Standup");
        assert_eq!(e.calendar, "Work");
        assert_eq!(e.kind, Kind::Other);
        assert_eq!(e.location.as_deref(), Some("Room 1"));
        assert!(!e.all_day);
        assert_eq!(e.start.with_timezone(&Utc), Utc.with_ymd_and_hms(2026, 11, 16, 9, 0, 0).unwrap());
        assert_eq!(e.end - e.start, chrono::Duration::minutes(15));
    }

    #[test]
    fn all_day_event_has_exclusive_end() {
        let e = event(r#"{"summary":"Offsite","start":{"date":"2026-11-16"},"end":{"date":"2026-11-18"}}"#).unwrap();
        assert!(e.all_day);
        assert_eq!(e.start, Local.with_ymd_and_hms(2026, 11, 16, 0, 0, 0).unwrap());
        assert_eq!(e.end, Local.with_ymd_and_hms(2026, 11, 18, 0, 0, 0).unwrap());
    }

    #[test]
    fn dropped_events() {
        let times = r#""start":{"dateTime":"2026-11-16T10:00:00Z"},"end":{"dateTime":"2026-11-16T11:00:00Z"}"#;
        assert!(event(&format!(r#"{{"status":"cancelled",{times}}}"#)).is_none());
        assert!(event(&format!(r#"{{"eventType":"workingLocation",{times}}}"#)).is_none());
        assert!(event(&format!(
            r#"{{{times},"attendees":[{{"email":"a@x","responseStatus":"accepted"}},{{"self":true,"responseStatus":"declined"}}]}}"#
        ))
        .is_none());
        assert!(event(r#"{"summary":"no times"}"#).is_none());
        // Someone else declining doesn't hide it; no title gets a placeholder.
        let kept = event(&format!(r#"{{{times},"attendees":[{{"responseStatus":"declined"}},{{"self":true,"responseStatus":"accepted"}}]}}"#)).unwrap();
        assert_eq!(kept.title, "(No title)");
    }

    #[test]
    fn office_rooms_keep_just_the_room() {
        assert_eq!(tidy_location("285W 5th-4FL-Room 306 (6)"), "Room 306");
        assert_eq!(tidy_location("285W 5th-3FL-Main Conference Room (311) (12)"), "Main Conference Room (311)");
        assert_eq!(tidy_location("285 W 5th - 4fl - Room 306 (6)"), "Room 306");
        assert_eq!(
            tidy_location("285W 5th-4FL-Room 306 (6), 285W 5th-3FL-Main Conference Room (311) (12)"),
            "Room 306, Main Conference Room (311)"
        );
        // Not the office, or no floor marker: unchanged.
        assert_eq!(tidy_location("Studio 4A · Video call"), "Studio 4A · Video call");
        assert_eq!(tidy_location("123 Main St-4FL-Room 1 (4)"), "123 Main St-4FL-Room 1 (4)");
        assert_eq!(tidy_location("285 W 5th St, New York"), "285 W 5th St, New York");
        // Applied when events are read.
        let e = event(
            r#"{"summary":"Sync","location":"285W 5th-4FL-Room 306 (6)",
                "start":{"dateTime":"2026-11-16T10:00:00Z"},"end":{"dateTime":"2026-11-16T11:00:00Z"}}"#,
        )
        .unwrap();
        assert_eq!(e.location.as_deref(), Some("Room 306"));
    }

    #[test]
    fn badge_rules_in_order() {
        let times = r#""start":{"dateTime":"2026-11-16T10:00:00Z"},"end":{"dateTime":"2026-11-16T11:00:00Z"}"#;
        let k = |extra: &str| event(&format!("{{{times},{extra}}}")).unwrap().kind;
        let me = |status: &str| format!(r#""attendees":[{{"email":"x@y","responseStatus":"accepted"}},{{"self":true,"responseStatus":"{status}"}}]"#);
        // Focus time wins, even though I organize it.
        assert_eq!(k(r#""eventType":"focusTime","organizer":{"self":true}"#), Kind::Focus);
        // I booked it: organizer is me (with or without guests).
        assert_eq!(k(r#""organizer":{"self":true}"#), Kind::Booked);
        assert_eq!(k(&format!(r#""organizer":{{"self":true}},{}"#, me("accepted"))), Kind::Booked);
        // Invitations I haven't accepted.
        assert_eq!(k(&format!(r#""organizer":{{"email":"boss@x"}},{}"#, me("needsAction"))), Kind::Pending);
        assert_eq!(k(&format!(r#""organizer":{{"email":"boss@x"}},{}"#, me("tentative"))), Kind::Pending);
        // The rest: accepted invitations, events on other people's / shared calendars.
        assert_eq!(k(&format!(r#""organizer":{{"email":"boss@x"}},{}"#, me("accepted"))), Kind::Other);
        assert_eq!(k(r#""organizer":{"email":"holidays@group"}"#), Kind::Other);
    }

    #[test]
    fn only_ticked_visible_calendars() {
        let list: Page<CalendarEntry> = serde_json::from_str(
            r#"{"items":[
                {"id":"a","summary":"A","selected":true},
                {"id":"b","summary":"B"},
                {"id":"c","summary":"C","selected":true,"hidden":true}]}"#,
        )
        .unwrap();
        let ids: Vec<_> = shown(list.items).into_iter().map(|c| c.id).collect();
        assert_eq!(ids, ["a"]);
    }

    #[test]
    fn fetch_pages_through_selected_calendars() {
        let (base, requests) = serve(vec![
            json_response(
                "200 OK",
                r#"{"items":[{"id":"me@example.com","summary":"Me","selected":true},
                             {"id":"team#holiday@group.v.calendar.google.com","summary":"Off"}]}"#,
            ),
            json_response(
                "200 OK",
                r#"{"items":[{"summary":"Second","start":{"dateTime":"2026-11-16T12:00:00Z"},"end":{"dateTime":"2026-11-16T13:00:00Z"}}],
                    "nextPageToken":"p2"}"#,
            ),
            json_response(
                "200 OK",
                r#"{"items":[{"summary":"First","start":{"dateTime":"2026-11-16T09:00:00Z"},"end":{"dateTime":"2026-11-16T10:00:00Z"}}]}"#,
            ),
        ]);
        let auth = auth_against(&base);
        let mut login = StoredLogin {
            refresh_token: "r".into(),
            access_token: "a".into(),
            expires_at: Utc::now().timestamp() + 3600,
            email: None,
        };
        let from = Local.with_ymd_and_hms(2026, 11, 14, 0, 0, 0).unwrap();
        let events = fetch(&auth, &mut login, from, from + chrono::Duration::days(14)).unwrap();

        let titles: Vec<_> = events.iter().map(|e| e.title.as_str()).collect();
        assert_eq!(titles, ["First", "Second"]); // merged across pages and sorted
        assert!(events.iter().all(|e| e.calendar == "Me"));

        assert!(requests.recv().unwrap().starts_with("GET /users/me/calendarList?fields="));
        let page1 = requests.recv().unwrap();
        assert!(page1.starts_with("GET /calendars/me@example.com/events?timeMin="), "{page1}");
        assert!(page1.contains("singleEvents=true"));
        assert!(!page1.contains("pageToken"));
        assert!(requests.recv().unwrap().contains("pageToken=p2"));
    }

    #[test]
    fn calendar_ids_are_percent_encoded() {
        let auth = auth_against("http://127.0.0.1:1");
        let url = auth.api_url(&["calendars", "team#holiday@group.v.calendar.google.com", "events"], &[]);
        assert_eq!(url, "http://127.0.0.1:1/calendars/team%23holiday@group.v.calendar.google.com/events");
    }
}
