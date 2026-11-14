//! Google OAuth for a desktop app: authorization code + PKCE with a loopback
//! redirect, and refreshing access tokens from the saved refresh token.
//!
//! Login: listen on 127.0.0.1 (random port), print Google's consent URL, wait
//! for the browser to be redirected back with `?code=…&state=…`, then exchange
//! the code (plus the PKCE verifier) for tokens. The OAuth bits (URL, PKCE,
//! token exchange/refresh/revoke) come from the `oauth2` crate; HTTP is `ureq`.
//!
//! The device login flow (show a code, approve on a phone) isn't usable: Google
//! doesn't allow Calendar scopes on it.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

use oauth2::basic::{BasicClient, BasicErrorResponseType};
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointNotSet, EndpointSet,
    PkceCodeChallenge, RedirectUrl, RefreshToken, RequestTokenError, RevocationUrl, Scope,
    StandardRevocableToken, TokenResponse, TokenUrl,
};

use super::ClientSecrets;
use super::store::StoredLogin;

/// Read-only access to calendars and their events (also covers the calendar list).
pub const SCOPE: &str = "https://www.googleapis.com/auth/calendar.readonly";

/// Refresh this long before the access token actually expires.
const EXPIRY_MARGIN_SECS: i64 = 60;

pub struct Endpoints {
    pub auth: &'static str,
    pub token: String,
    pub revoke: String,
    pub api: String,
}

impl Endpoints {
    pub fn google() -> Self {
        Endpoints {
            auth: "https://accounts.google.com/o/oauth2/v2/auth",
            token: "https://oauth2.googleapis.com/token".into(),
            revoke: "https://oauth2.googleapis.com/revoke".into(),
            api: "https://www.googleapis.com/calendar/v3".into(),
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum AuthError {
    /// The saved login no longer works (revoked, expired, or client changed): log in again.
    Revoked,
    /// Anything else (offline, Google down, API not enabled…): keep the login, retry later.
    Other(String),
}

// Typestate: auth URL, device URL, introspection URL, revocation URL, token URL.
type Client = BasicClient<EndpointSet, EndpointNotSet, EndpointNotSet, EndpointSet, EndpointSet>;

pub struct Auth {
    client: Client,
    agent: ureq::Agent,
    api: String,
}

impl Auth {
    pub fn new(secrets: &ClientSecrets, endpoints: Endpoints) -> Result<Self, String> {
        let url_err = |e: oauth2::url::ParseError| e.to_string();
        let client = BasicClient::new(ClientId::new(secrets.client_id.clone()))
            .set_client_secret(ClientSecret::new(secrets.client_secret.clone()))
            .set_auth_uri(AuthUrl::new(endpoints.auth.into()).map_err(url_err)?)
            .set_token_uri(TokenUrl::new(endpoints.token).map_err(url_err)?)
            .set_revocation_url(RevocationUrl::new(endpoints.revoke).map_err(url_err)?);
        let agent = ureq::Agent::config_builder()
            // 4xx/5xx are normal responses: `oauth2` needs to read Google's error body
            // (e.g. invalid_grant), and API calls report Google's message.
            .http_status_as_error(false)
            // OAuth endpoints must not be redirected elsewhere; the API doesn't redirect.
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(30)))
            .user_agent("my-thing")
            .build()
            .into();
        Ok(Auth { client, agent, api: endpoints.api })
    }

    /// Interactive login. Calls `on_url` with the URL to open, waits for the browser
    /// to come back, calls `on_redirect`, then exchanges the code for tokens.
    pub fn login(
        &self,
        on_url: impl FnOnce(&str),
        on_redirect: impl FnOnce(),
    ) -> Result<StoredLogin, String> {
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("can't listen for the redirect: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let redirect = RedirectUrl::new(format!("http://127.0.0.1:{port}")).map_err(|e| e.to_string())?;
        let client = self.client.clone().set_redirect_uri(redirect);

        let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
        let (url, state) = client
            .authorize_url(CsrfToken::new_random)
            .add_scope(Scope::new(SCOPE.into()))
            // offline + consent: always get a refresh token, even on a repeat login.
            .add_extra_param("access_type", "offline")
            .add_extra_param("prompt", "consent")
            .set_pkce_challenge(challenge)
            .url();
        on_url(url.as_str());

        let code = wait_for_code(&listener, state.secret())?;
        on_redirect();

        let token = client
            .exchange_code(AuthorizationCode::new(code))
            .set_pkce_verifier(verifier)
            .request(&self.http())
            .map_err(|e| describe(&e))?;
        let refresh_token = token
            .refresh_token()
            .ok_or("Google didn't return a refresh token")?
            .secret()
            .clone();
        Ok(StoredLogin {
            refresh_token,
            access_token: token.access_token().secret().clone(),
            expires_at: expires_at(token.expires_in()),
            email: None,
        })
    }

    /// A valid access token, refreshing it (and updating `login`) if it has expired.
    pub fn access_token(&self, login: &mut StoredLogin) -> Result<String, AuthError> {
        if !needs_refresh(login.expires_at, now()) {
            return Ok(login.access_token.clone());
        }
        let token = self
            .client
            .exchange_refresh_token(&RefreshToken::new(login.refresh_token.clone()))
            .request(&self.http())
            .map_err(|e| match &e {
                RequestTokenError::ServerResponse(r)
                    if matches!(r.error(), BasicErrorResponseType::InvalidGrant) =>
                {
                    AuthError::Revoked
                }
                _ => AuthError::Other(describe(&e)),
            })?;
        login.access_token = token.access_token().secret().clone();
        login.expires_at = expires_at(token.expires_in());
        // Google normally keeps the same refresh token; take a new one if it sends one.
        if let Some(r) = token.refresh_token() {
            login.refresh_token = r.secret().clone();
        }
        Ok(login.access_token.clone())
    }

    /// A Calendar API URL. Each segment and query value is percent-encoded
    /// (calendar ids contain `@` and `#`).
    pub fn api_url(&self, segments: &[&str], query: &[(&str, &str)]) -> String {
        let mut url = url::Url::parse(&self.api).expect("API base is a valid URL");
        url.path_segments_mut().expect("API base can have a path").pop_if_empty().extend(segments);
        if !query.is_empty() {
            url.query_pairs_mut().extend_pairs(query);
        }
        url.into()
    }

    /// GET a Calendar API URL (from [`Auth::api_url`]) and parse the JSON.
    /// On 401 the access token is refreshed and the request retried once.
    pub fn get_json<T: serde::de::DeserializeOwned>(
        &self,
        login: &mut StoredLogin,
        url: &str,
    ) -> Result<T, AuthError> {
        for attempt in 0..2 {
            let token = self.access_token(login)?;
            let mut resp = self
                .agent
                .get(url)
                .header("Authorization", format!("Bearer {token}"))
                .call()
                .map_err(|e| AuthError::Other(e.to_string()))?;
            let status = resp.status().as_u16();
            let body = resp.body_mut().read_to_string().map_err(|e| AuthError::Other(e.to_string()))?;
            match status {
                200..=299 => return serde_json::from_str(&body).map_err(|e| AuthError::Other(e.to_string())),
                401 if attempt == 0 => login.expires_at = 0, // force a refresh, then retry
                _ => return Err(AuthError::Other(api_error(status, &body))),
            }
        }
        Err(AuthError::Other("still unauthorized after refreshing the token".into()))
    }

    /// The logged-in account's email (the id of its primary calendar). Also proves
    /// the token works and the Calendar API is enabled.
    pub fn whoami(&self, login: &mut StoredLogin) -> Result<String, AuthError> {
        #[derive(serde::Deserialize)]
        struct Primary {
            id: String,
        }
        let url = self.api_url(&["calendars", "primary"], &[]);
        Ok(self.get_json::<Primary>(login, &url)?.id)
    }

    /// Best effort: tell Google to forget this login.
    pub fn revoke(&self, login: &StoredLogin) -> Result<(), String> {
        let token = StandardRevocableToken::RefreshToken(RefreshToken::new(login.refresh_token.clone()));
        self.client
            .revoke_token(token)
            .map_err(|e| e.to_string())?
            .request(&self.http())
            .map_err(|e| describe(&e))
    }

    /// `oauth2`'s HTTP hook, backed by our ureq agent.
    fn http(&self) -> impl Fn(oauth2::HttpRequest) -> Result<oauth2::HttpResponse, ureq::Error> + '_ {
        move |req| {
            let (parts, mut body) = self.agent.run(req)?.into_parts();
            Ok(oauth2::http::Response::from_parts(parts, body.read_to_vec()?))
        }
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

fn needs_refresh(expires_at: i64, now: i64) -> bool {
    now >= expires_at - EXPIRY_MARGIN_SECS
}

fn expires_at(expires_in: Option<Duration>) -> i64 {
    // Google always sends expires_in; without it, assume the usual hour.
    now() + expires_in.map_or(3600, |d| d.as_secs() as i64)
}

/// A token-endpoint error as one readable line (Google's error code + description).
fn describe<RE: std::error::Error + 'static, T: oauth2::ErrorResponse + 'static>(
    e: &RequestTokenError<RE, T>,
) -> String {
    match e {
        RequestTokenError::ServerResponse(r) => format!("Google: {r}"),
        RequestTokenError::Request(re) => format!("request failed: {re}"),
        RequestTokenError::Parse(pe, _) => format!("unexpected reply from Google: {pe}"),
        RequestTokenError::Other(s) => s.clone(),
    }
}

/// Google API errors look like {"error": {"code": 403, "message": "..."}}.
fn api_error(status: u16, body: &str) -> String {
    let msg = serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v["error"]["message"].as_str().map(String::from));
    match msg {
        Some(m) => format!("Google API {status}: {m}"),
        None => format!("Google API returned HTTP {status}"),
    }
}

// ---------- The loopback redirect ----------

#[derive(Debug, PartialEq)]
struct Redirect {
    code: String,
    state: String,
}

/// Accepts connections until the browser arrives with Google's redirect.
/// Other requests (e.g. /favicon.ico) get a 404; a wrong `state` is ignored.
fn wait_for_code(listener: &TcpListener, expected_state: &str) -> Result<String, String> {
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else { continue };
        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
        let Some(line) = read_request(&stream) else { continue };
        match parse_redirect(&line) {
            None => respond(&mut stream, "404 Not Found", "Not found."),
            Some(Ok(r)) if r.state != expected_state => {
                respond(&mut stream, "400 Bad Request", "This login link is out of date. Use the newest link from the terminal.");
            }
            Some(Ok(r)) => {
                respond(&mut stream, "200 OK", "Logged in to my-thing. You can close this tab.");
                return Ok(r.code);
            }
            Some(Err(e)) => {
                respond(&mut stream, "200 OK", &format!("Login failed: {e}. A new link is in the terminal."));
                return Err(e);
            }
        }
    }
    Err("redirect listener stopped".into())
}

/// The request line; the headers are read and dropped so the browser sees a clean reply.
pub(super) fn read_request(stream: &TcpStream) -> Option<String> {
    let mut reader = BufReader::new(stream);
    let mut first = String::new();
    reader.read_line(&mut first).ok()?;
    let mut header = String::new();
    while reader.read_line(&mut header).ok()? > 0 && header.trim() != "" {
        header.clear();
    }
    Some(first)
}

fn respond(stream: &mut TcpStream, status: &str, text: &str) {
    let body = format!(
        "<!doctype html><meta charset=utf-8><title>my-thing</title>\
         <p style=\"font:20px system-ui;margin:3em\">{text}</p>"
    );
    let _ = write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
}

/// `GET /?state=…&code=…&scope=… HTTP/1.1` → Some(Ok). `?error=access_denied` → Some(Err).
/// Anything else isn't the redirect → None.
fn parse_redirect(request_line: &str) -> Option<Result<Redirect, String>> {
    let mut parts = request_line.split_whitespace();
    if parts.next()? != "GET" {
        return None;
    }
    let url = url::Url::parse(&format!("http://127.0.0.1{}", parts.next()?)).ok()?;
    if url.path() != "/" {
        return None;
    }
    let get = |key: &str| url.query_pairs().find(|(k, _)| k == key).map(|(_, v)| v.into_owned());
    if let Some(err) = get("error") {
        return Some(Err(match err.as_str() {
            "access_denied" => "access was denied in the browser".into(),
            other => format!("Google returned {other}"),
        }));
    }
    Some(Ok(Redirect { code: get("code")?, state: get("state")? }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::google::testutil::{auth_against, json_response, serve};

    #[test]
    fn redirect_with_code() {
        let r = parse_redirect("GET /?state=abc&code=4%2F0Ab&scope=https%3A%2F%2Fx HTTP/1.1\r\n");
        assert_eq!(r, Some(Ok(Redirect { code: "4/0Ab".into(), state: "abc".into() })));
    }

    #[test]
    fn redirect_with_error() {
        let r = parse_redirect("GET /?error=access_denied&state=abc HTTP/1.1");
        assert_eq!(r, Some(Err("access was denied in the browser".into())));
    }

    #[test]
    fn other_requests_are_not_the_redirect() {
        assert_eq!(parse_redirect("GET /favicon.ico HTTP/1.1"), None);
        assert_eq!(parse_redirect("GET / HTTP/1.1"), None);
        assert_eq!(parse_redirect("POST /?code=x&state=y HTTP/1.1"), None);
        assert_eq!(parse_redirect(""), None);
    }

    #[test]
    fn api_url_keeps_the_base_path() {
        let secrets = ClientSecrets { client_id: "id".into(), client_secret: "s".into() };
        let auth = Auth::new(&secrets, Endpoints::google()).unwrap();
        assert_eq!(
            auth.api_url(&["users", "me", "calendarList"], &[("fields", "items(id),nextPageToken")]),
            "https://www.googleapis.com/calendar/v3/users/me/calendarList?fields=items%28id%29%2CnextPageToken"
        );
    }

    #[test]
    fn refresh_margin() {
        assert!(!needs_refresh(1000, 900));
        assert!(needs_refresh(1000, 940)); // within the 60 s margin
        assert!(needs_refresh(1000, 2000));
    }

    #[test]
    fn api_error_message() {
        let body = r#"{"error":{"code":403,"message":"Google Calendar API has not been used in project 1 before or it is disabled."}}"#;
        assert_eq!(
            api_error(403, body),
            "Google API 403: Google Calendar API has not been used in project 1 before or it is disabled."
        );
        assert_eq!(api_error(502, "<html>"), "Google API returned HTTP 502");
    }

    fn expired_login() -> StoredLogin {
        StoredLogin { refresh_token: "r1".into(), access_token: "old".into(), expires_at: 0, email: None }
    }

    /// Plays the browser: reads the consent URL, then hits the loopback redirect.
    #[test]
    fn loopback_login_end_to_end() {
        let (base, requests) = serve(vec![json_response(
            "200 OK",
            r#"{"access_token":"at","expires_in":3599,"refresh_token":"rt","token_type":"Bearer"}"#,
        )]);
        let auth = auth_against(&base);
        let browser = std::sync::Mutex::new(None);
        let login = auth
            .login(
                |consent| {
                    let url = url::Url::parse(consent).unwrap();
                    let q = |k: &str| url.query_pairs().find(|(n, _)| n == k).unwrap().1.into_owned();
                    assert_eq!(q("scope"), SCOPE);
                    assert_eq!(q("access_type"), "offline");
                    assert_eq!(q("code_challenge_method"), "S256");
                    let (redirect, state) = (q("redirect_uri"), q("state"));
                    assert!(redirect.starts_with("http://127.0.0.1:"));
                    *browser.lock().unwrap() = Some(std::thread::spawn(move || {
                        let get = |path: String| {
                            let agent: ureq::Agent = ureq::Agent::config_builder()
                                .http_status_as_error(false)
                                .build()
                                .into();
                            agent.get(format!("{redirect}{path}")).call().unwrap().status().as_u16()
                        };
                        assert_eq!(get("/favicon.ico".into()), 404);
                        assert_eq!(get("/?code=evil&state=wrong".into()), 400);
                        assert_eq!(get(format!("/?code=the-code&state={state}")), 200);
                    }));
                },
                || {},
            )
            .unwrap();
        browser.lock().unwrap().take().unwrap().join().unwrap();
        assert_eq!(login.refresh_token, "rt");
        assert_eq!(login.access_token, "at");
        assert_eq!(requests.recv().unwrap(), "POST /token HTTP/1.1");
    }

    #[test]
    fn invalid_grant_means_revoked() {
        let (base, _) = serve(vec![json_response(
            "400 Bad Request",
            r#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#,
        )]);
        let mut login = expired_login();
        assert_eq!(auth_against(&base).access_token(&mut login), Err(AuthError::Revoked));
    }

    #[test]
    fn expired_token_is_refreshed_then_api_called() {
        let (base, requests) = serve(vec![
            json_response("200 OK", r#"{"access_token":"new","expires_in":3599,"token_type":"Bearer"}"#),
            json_response("200 OK", r#"{"id":"me@example.com","summary":"Me"}"#),
        ]);
        let mut login = expired_login();
        let email = auth_against(&base).whoami(&mut login).unwrap();
        assert_eq!(email, "me@example.com");
        assert_eq!(login.access_token, "new");
        assert_eq!(login.refresh_token, "r1"); // kept when Google doesn't send a new one
        assert!(login.expires_at > now() + 3000);
        assert_eq!(requests.recv().unwrap(), "POST /token HTTP/1.1");
        assert_eq!(requests.recv().unwrap(), "GET /calendars/primary HTTP/1.1");
    }

    #[test]
    fn unauthorized_api_call_refreshes_and_retries_once() {
        let (base, _) = serve(vec![
            json_response("401 Unauthorized", r#"{"error":{"code":401,"message":"Invalid Credentials"}}"#),
            json_response("200 OK", r#"{"access_token":"new","expires_in":3599,"token_type":"Bearer"}"#),
            json_response("200 OK", r#"{"id":"me@example.com"}"#),
        ]);
        let mut login = StoredLogin { expires_at: now() + 3600, ..expired_login() };
        assert_eq!(auth_against(&base).whoami(&mut login).unwrap(), "me@example.com");
        assert_eq!(login.access_token, "new");
    }
}
