//! The saved Google login, so the app stays logged in across restarts.
//!
//! One JSON file, readable only by its owner (it holds a refresh token, which
//! grants read access to the calendar until revoked). Written to a temporary
//! file and renamed, so a crash mid-write never leaves a half-written login.

use std::fs;
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StoredLogin {
    /// Long-lived; used to get new access tokens.
    pub refresh_token: String,
    /// Short-lived (about an hour); sent with each API request.
    pub access_token: String,
    /// When `access_token` expires, in Unix seconds.
    pub expires_at: i64,
    /// The account's primary calendar id (its email), once known.
    #[serde(default)]
    pub email: Option<String>,
}

/// None if there's no saved login. A corrupt file is reported and treated as none.
pub fn load(path: &Path) -> Option<StoredLogin> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return None,
        Err(e) => {
            eprintln!("google: can't read {}: {e}", path.display());
            return None;
        }
    };
    match serde_json::from_str(&text) {
        Ok(login) => Some(login),
        Err(e) => {
            eprintln!("google: ignoring unreadable {}: {e}", path.display());
            None
        }
    }
}

pub fn save(path: &Path, login: &StoredLogin) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    f.write_all(serde_json::to_string_pretty(login)?.as_bytes())?;
    f.sync_all()?;
    fs::rename(&tmp, path)
}

pub fn delete(path: &Path) {
    match fs::remove_file(path) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => eprintln!("google: can't delete {}: {e}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    /// A fresh directory under the system temp dir, removed when dropped.
    struct TempDir(PathBuf);
    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("my-thing-test-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            TempDir(dir)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn login() -> StoredLogin {
        StoredLogin {
            refresh_token: "r".into(),
            access_token: "a".into(),
            expires_at: 1_800_000_000,
            email: Some("me@example.com".into()),
        }
    }

    #[test]
    fn round_trip_creates_dirs_and_is_private() {
        let dir = TempDir::new("store-rt");
        let path = dir.0.join("nested/google-login.json");
        save(&path, &login()).unwrap();
        assert_eq!(load(&path), Some(login()));
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn missing_or_corrupt_is_none() {
        let dir = TempDir::new("store-bad");
        let path = dir.0.join("google-login.json");
        assert_eq!(load(&path), None);
        fs::create_dir_all(&dir.0).unwrap();
        fs::write(&path, "{not json").unwrap();
        assert_eq!(load(&path), None);
    }

    #[test]
    fn delete_removes_and_tolerates_missing() {
        let dir = TempDir::new("store-del");
        let path = dir.0.join("google-login.json");
        save(&path, &login()).unwrap();
        delete(&path);
        assert_eq!(load(&path), None);
        delete(&path); // no panic, no error
    }
}
