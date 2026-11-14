//! Google account access for the Calendar.
//!
//! - `auth`: OAuth login (browser + loopback redirect), token refresh, API GETs.
//! - `store`: the saved login, so restarts don't ask again.
//! - `calendar`: Calendar API responses → [`crate::calendar::Event`]s.
//! - [`spawn`]: background thread that logs in, then fetches events every few
//!   minutes, reporting [`Msg`]s to the UI thread.
//!
//! Files (outside the repo):
//! - `~/.config/my-thing/google-client.json`: the OAuth client downloaded from Google Cloud.
//! - `~/.local/state/my-thing/google-login.json`: the saved login (owner-only).

pub mod auth;
pub mod calendar;
pub mod store;

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::time::Duration;

use chrono::{DateTime, Local, NaiveTime};
use serde::Deserialize;

use crate::calendar::Event;
use auth::{Auth, AuthError, Endpoints};
use store::StoredLogin;

/// How often events are fetched when everything works.
const POLL: Duration = Duration::from_secs(5 * 60);
/// First retry after a network / API error (the saved login is kept); doubles up to POLL.
const RETRY_MIN: Duration = Duration::from_secs(30);
/// Days fetched, from the start of today.
const WINDOW_DAYS: i64 = 14;

/// From the UI to the Google thread.
pub enum Cmd {
    /// Fetch now instead of waiting for the next poll (or retry).
    RefreshNow,
}

/// From the Google thread to the UI.
#[derive(Debug)]
pub enum Msg {
    Status(Status),
    /// A complete fetch of every shown calendar.
    Synced { events: Vec<Event>, at: DateTime<Local> },
    /// A fetch failed; the UI keeps the previous events.
    SyncFailed(String),
}

/// What the Calendar page shows about the Google account.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    /// No (valid) OAuth client file.
    NotSetUp(String),
    /// Checking the saved login.
    Connecting,
    /// Login URL printed; waiting for the browser.
    WaitingForBrowser,
    /// Browser came back; exchanging the code.
    LoggingIn,
    LoggedIn { email: String },
    /// Will retry by itself.
    Failed(String),
}

impl Status {
    pub fn label(&self) -> String {
        match self {
            Status::NotSetUp(why) => format!("Google login not set up: {why}"),
            Status::Connecting => "Connecting to Google…".into(),
            Status::WaitingForBrowser => "Not logged in: open the link printed in the terminal".into(),
            Status::LoggingIn => "Logging in…".into(),
            Status::LoggedIn { email } => format!("Logged in as {email}"),
            Status::Failed(e) => e.clone(),
        }
    }
}

/// The `installed` (Desktop app) section of the client JSON from Google Cloud.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ClientSecrets {
    pub client_id: String,
    pub client_secret: String,
}

fn parse_client(text: &str) -> Result<ClientSecrets, String> {
    #[derive(Deserialize)]
    struct File {
        installed: Option<ClientSecrets>,
        web: Option<ClientSecrets>,
    }
    let f: File = serde_json::from_str(text).map_err(|e| format!("not a Google client file ({e})"))?;
    match (f.installed, f.web) {
        (Some(c), _) => Ok(c),
        (None, Some(_)) => Err("it's a Web client; create a \"Desktop app\" client instead".into()),
        (None, None) => Err("not a Google OAuth client file".into()),
    }
}

pub fn client_path() -> Option<PathBuf> {
    xdg_dir("XDG_CONFIG_HOME", ".config").map(|d| d.join("my-thing/google-client.json"))
}

pub fn login_path() -> Option<PathBuf> {
    xdg_dir("XDG_STATE_HOME", ".local/state").map(|d| d.join("my-thing/google-login.json"))
}

fn xdg_dir(var: &str, fallback: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(fallback)))
}

/// Starts the Google thread. With `logout`, the saved login is revoked and deleted first.
pub fn spawn(logout: bool) -> (Sender<Cmd>, Receiver<Msg>) {
    let (cmd_tx, cmd_rx) = mpsc::channel();
    let (msg_tx, msg_rx) = mpsc::channel();
    std::thread::spawn(move || run(logout, cmd_rx, msg_tx));
    (cmd_tx, msg_rx)
}

enum Outcome {
    /// The login stopped working: log in again.
    Revoked,
    /// The UI is gone.
    Quit,
}

fn run(logout: bool, cmds: Receiver<Cmd>, tx: Sender<Msg>) {
    let send = |s: Status| {
        eprintln!("google: {}", s.label());
        let _ = tx.send(Msg::Status(s));
    };
    let (Some(client_path), Some(login_path)) = (client_path(), login_path()) else {
        send(Status::NotSetUp("$HOME is not set".into()));
        return;
    };
    let auth = match load_client(&client_path).and_then(|c| Auth::new(&c, Endpoints::google())) {
        Ok(a) => a,
        Err(why) => {
            send(Status::NotSetUp(why));
            return;
        }
    };

    let mut login = store::load(&login_path);
    if logout {
        if let Some(l) = login.take() {
            match auth.revoke(&l) {
                Ok(()) => eprintln!("google: login revoked"),
                Err(e) => eprintln!("google: revoking failed ({e}); deleting the saved login anyway"),
            }
        }
        store::delete(&login_path);
    }

    let mut retry = RETRY_MIN;
    loop {
        let mut current = match login.take() {
            Some(l) => {
                send(Status::Connecting);
                l
            }
            None => match auth.login(print_login_url_and(&send), || send(Status::LoggingIn)) {
                Ok(l) => l,
                Err(e) => {
                    send(Status::Failed(format!("Login failed: {e}")));
                    continue; // straight to a fresh link
                }
            },
        };
        match auth.whoami(&mut current) {
            Ok(email) => {
                retry = RETRY_MIN;
                current.email = Some(email.clone());
                save(&login_path, &current);
                send(Status::LoggedIn { email });
                match sync_loop(&auth, &mut current, &login_path, &cmds, &tx) {
                    Outcome::Quit => return,
                    Outcome::Revoked => {
                        store::delete(&login_path);
                        send(Status::Failed("Google login expired or was revoked; log in again".into()));
                    }
                }
            }
            Err(AuthError::Revoked) => {
                store::delete(&login_path);
                send(Status::Failed("Google login expired or was revoked; log in again".into()));
            }
            Err(AuthError::Other(e)) => {
                save(&login_path, &current); // keep any refreshed token
                send(Status::Failed(format!("Can't reach Google: {e}")));
                login = Some(current);
                if wait(&cmds, retry).is_err() {
                    return;
                }
                retry = (retry * 2).min(POLL);
            }
        }
    }
}

/// Logged in: fetch every POLL (sooner after an error, or on [`Cmd::RefreshNow`]).
fn sync_loop(
    auth: &Auth,
    login: &mut StoredLogin,
    login_path: &Path,
    cmds: &Receiver<Cmd>,
    tx: &Sender<Msg>,
) -> Outcome {
    // Presses made while logging in are covered by the first fetch.
    while cmds.try_recv().is_ok() {}
    let mut retry = RETRY_MIN;
    loop {
        let now = Local::now();
        let from = now
            .date_naive()
            .and_time(NaiveTime::MIN)
            .and_local_timezone(Local)
            .earliest()
            .unwrap_or(now);
        let before = login.clone();
        let result = calendar::fetch(auth, login, from, from + chrono::Duration::days(WINDOW_DAYS));
        if *login != before {
            save(login_path, login); // the access token was refreshed
        }
        let (msg, delay) = match result {
            Ok(events) => {
                retry = RETRY_MIN;
                (Msg::Synced { events, at: now }, POLL)
            }
            Err(AuthError::Revoked) => return Outcome::Revoked,
            Err(AuthError::Other(e)) => {
                eprintln!("google: sync failed: {e}");
                let d = retry;
                retry = (retry * 2).min(POLL);
                (Msg::SyncFailed(e), d)
            }
        };
        if tx.send(msg).is_err() || wait(cmds, delay).is_err() {
            return Outcome::Quit;
        }
    }
}

/// Waits `delay`, or less if the UI asks for a refresh. Err when the UI is gone.
fn wait(cmds: &Receiver<Cmd>, delay: Duration) -> Result<(), ()> {
    match cmds.recv_timeout(delay) {
        Ok(Cmd::RefreshNow) => {
            while cmds.try_recv().is_ok() {} // several presses, one fetch
            Ok(())
        }
        Err(RecvTimeoutError::Timeout) => Ok(()),
        Err(RecvTimeoutError::Disconnected) => Err(()),
    }
}

fn load_client(path: &Path) -> Result<ClientSecrets, String> {
    let text = std::fs::read_to_string(path).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => format!("no {} (see README)", path.display()),
        _ => format!("can't read {}: {e}", path.display()),
    })?;
    parse_client(&text).map_err(|e| format!("{}: {e}", path.display()))
}

fn save(path: &Path, login: &StoredLogin) {
    if let Err(e) = store::save(path, login) {
        eprintln!("google: can't save the login to {}: {e} (you'll have to log in again next time)", path.display());
    }
}

/// The login URL goes to the terminal: copy it into a browser on this computer.
fn print_login_url_and(send: &impl Fn(Status)) -> impl FnOnce(&str) + '_ {
    move |url| {
        eprintln!("\n=== Google login ===\nOpen this link in a browser on this computer:\n\n{url}\n");
        send(Status::WaitingForBrowser);
    }
}

/// Test helpers shared by the google modules: a fake HTTP server.
#[cfg(test)]
pub(crate) mod testutil {
    use std::io::Write;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::Duration;

    use super::ClientSecrets;
    use super::auth::{Auth, Endpoints, read_request};

    /// Answers each connection with the next canned response; reports each request's first line.
    pub fn serve(responses: Vec<String>) -> (String, mpsc::Receiver<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            for resp in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let first = read_request(&stream).unwrap();
                // Drain a form body if there is one (token requests are small).
                stream.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
                let _ = std::io::Read::read(&mut stream, &mut [0u8; 4096]);
                stream.write_all(resp.as_bytes()).unwrap();
                let _ = tx.send(first.trim().to_string()); // the test may not be listening
            }
        });
        (base, rx)
    }

    pub fn json_response(status: &str, body: &str) -> String {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }

    pub fn auth_against(base: &str) -> Auth {
        let secrets = ClientSecrets { client_id: "id".into(), client_secret: "secret".into() };
        let endpoints = Endpoints {
            auth: "https://accounts.google.com/o/oauth2/v2/auth",
            token: format!("{base}/token"),
            revoke: format!("{base}/revoke"),
            api: base.to_string(),
        };
        Auth::new(&secrets, endpoints).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_client_file() {
        let text = r#"{"installed":{"client_id":"123.apps.googleusercontent.com","project_id":"p",
            "auth_uri":"https://accounts.google.com/o/oauth2/auth","token_uri":"https://oauth2.googleapis.com/token",
            "client_secret":"GOCSPX-x","redirect_uris":["http://localhost"]}}"#;
        assert_eq!(
            parse_client(text),
            Ok(ClientSecrets {
                client_id: "123.apps.googleusercontent.com".into(),
                client_secret: "GOCSPX-x".into()
            })
        );
    }

    #[test]
    fn web_client_file_is_rejected_with_a_hint() {
        let text = r#"{"web":{"client_id":"a","client_secret":"b"}}"#;
        assert!(parse_client(text).unwrap_err().contains("Desktop app"));
        assert!(parse_client("{}").is_err());
        assert!(parse_client("nope").is_err());
    }
}
