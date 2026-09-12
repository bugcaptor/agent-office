//! Local storage for the most recently authenticated remote connection.
//!
//! Errors deliberately use fixed codes: neither a server address nor a token is
//! suitable for logs or IPC error strings.
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};
use tauri::{AppHandle, Manager};

const FILE_NAME: &str = "remote-connection.json";
const LOAD_ERROR: &str = "remote-connection-load-failed";
const SAVE_ERROR: &str = "remote-connection-save-failed";

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct StoredConnection {
    pub url: String,
    pub token: String,
}

fn path(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|dir| dir.join(FILE_NAME))
        .map_err(|_| String::from(LOAD_ERROR))
}

pub fn load(app: &AppHandle) -> Result<Option<StoredConnection>, String> {
    load_from_path(&path(app)?)
}

pub fn save(app: &AppHandle, url: &str, token: &str) -> Result<(), String> {
    super::transport::socket_url(url).map_err(|_| String::from(SAVE_ERROR))?;
    if token.trim().is_empty() {
        return Err(SAVE_ERROR.into());
    }
    let target = app
        .path()
        .app_data_dir()
        .map(|dir| dir.join(FILE_NAME))
        .map_err(|_| String::from(SAVE_ERROR))?;
    save_to_path(
        &target,
        &StoredConnection {
            url: url.trim().into(),
            token: token.trim().into(),
        },
    )
}

fn load_from_path(path: &Path) -> Result<Option<StoredConnection>, String> {
    let contents = match fs::read(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(LOAD_ERROR.into()),
    };
    let connection: StoredConnection = serde_json::from_slice(&contents).map_err(|_| LOAD_ERROR)?;
    if connection.url.trim().is_empty() || connection.token.trim().is_empty() {
        return Err(LOAD_ERROR.into());
    }
    super::transport::socket_url(&connection.url).map_err(|_| String::from(LOAD_ERROR))?;
    Ok(Some(connection))
}

fn save_to_path(path: &Path, connection: &StoredConnection) -> Result<(), String> {
    let parent = path.parent().ok_or(SAVE_ERROR)?;
    fs::create_dir_all(parent).map_err(|_| SAVE_ERROR)?;
    let bytes = serde_json::to_vec(connection).map_err(|_| SAVE_ERROR)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|_| SAVE_ERROR)?;
    set_private_permissions(temporary.path()).map_err(|_| SAVE_ERROR)?;
    use std::io::Write;
    temporary.write_all(&bytes).map_err(|_| SAVE_ERROR)?;
    temporary.flush().map_err(|_| SAVE_ERROR)?;
    temporary.persist(path).map_err(|_| SAVE_ERROR)?;
    Ok(())
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_private_permissions(_: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connection(url: &str, token: &str) -> StoredConnection {
        StoredConnection {
            url: url.into(),
            token: token.into(),
        }
    }

    #[test]
    fn round_trips_connection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        let expected = connection("https://office.example", "secret-token");
        save_to_path(&path, &expected).unwrap();
        assert!(load_from_path(&path).unwrap() == Some(expected));
    }

    #[test]
    fn replacement_keeps_only_new_connection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        save_to_path(&path, &connection("https://old.example", "old-token")).unwrap();
        let expected = connection("https://new.example", "new-token");
        save_to_path(&path, &expected).unwrap();
        assert!(load_from_path(&path).unwrap() == Some(expected));
    }

    #[test]
    fn missing_connection_is_none() {
        let dir = tempfile::tempdir().unwrap();
        assert!(load_from_path(&dir.path().join(FILE_NAME))
            .unwrap()
            .is_none());
    }

    #[test]
    fn malformed_connection_uses_redacted_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        fs::write(
            &path,
            br#"{\"url\":\"https://private.example\",\"token\":\"leak-me\""#,
        )
        .unwrap();
        let error = match load_from_path(&path) {
            Err(error) => error,
            Ok(_) => panic!("malformed connection must fail"),
        };
        assert_eq!(error, LOAD_ERROR);
        assert!(!error.contains("private.example"));
        assert!(!error.contains("leak-me"));
    }

    #[test]
    fn invalid_saved_values_use_redacted_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        fs::write(
            &path,
            br#"{"url":"ftp://private.example","token":"leak-me"}"#,
        )
        .unwrap();
        let error = match load_from_path(&path) {
            Err(error) => error,
            Ok(_) => panic!("invalid connection must fail"),
        };
        assert_eq!(error, LOAD_ERROR);
    }

    #[cfg(unix)]
    #[test]
    fn stored_file_is_owner_read_write_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(FILE_NAME);
        save_to_path(&path, &connection("https://office.example", "secret")).unwrap();
        assert_eq!(
            fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
