use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::CredentialUpdateError;

const DOTENV_TEMP_ATTEMPTS: usize = 5;

#[cfg(not(test))]
pub(super) fn persist_fields_to_dotenv(
    fields: &[(String, String)],
) -> Result<(), CredentialUpdateError> {
    let path = env_file_path()?;
    persist_fields_to_dotenv_path(&path, fields)
}

pub(super) fn persist_fields_to_dotenv_path(
    path: &Path,
    fields: &[(String, String)],
) -> Result<(), CredentialUpdateError> {
    let original = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(CredentialUpdateError::Persist(error)),
    };
    let updated = upsert_dotenv_text(&original, fields);
    write_dotenv_atomic(path, &updated).map_err(CredentialUpdateError::Persist)
}

#[cfg(not(test))]
pub(super) fn remove_fields_from_dotenv(fields: &[String]) -> Result<(), CredentialUpdateError> {
    let path = env_file_path()?;
    remove_fields_from_dotenv_path(&path, fields)
}

#[cfg(test)]
#[allow(clippy::unnecessary_wraps)]
pub(super) fn remove_fields_from_dotenv(_fields: &[String]) -> Result<(), CredentialUpdateError> {
    Ok(())
}

pub(super) fn remove_fields_from_dotenv_path(
    path: &Path,
    fields: &[String],
) -> Result<(), CredentialUpdateError> {
    let original = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(CredentialUpdateError::Persist(error)),
    };
    let updated = remove_dotenv_fields(&original, fields);
    write_dotenv_atomic(path, &updated).map_err(CredentialUpdateError::Persist)
}

#[cfg(test)]
#[allow(clippy::unnecessary_wraps)]
pub(super) fn persist_fields_to_dotenv(
    fields: &[(String, String)],
) -> Result<(), CredentialUpdateError> {
    let _ = fields;
    Ok(())
}

#[cfg(not(test))]
pub(super) fn env_file_path() -> Result<PathBuf, CredentialUpdateError> {
    let cwd = std::env::current_dir().map_err(CredentialUpdateError::Persist)?;
    for dir in cwd.ancestors() {
        if dir.join(".env").exists() || dir.join(".env.example").exists() {
            return Ok(dir.join(".env"));
        }
    }
    Ok(cwd.join(".env"))
}

pub(super) fn write_dotenv_atomic(path: &Path, contents: &str) -> std::io::Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }
    let mut last_collision = None;
    for _ in 0..DOTENV_TEMP_ATTEMPTS {
        let temp_path = dotenv_temp_path(path);
        match write_dotenv_temp_then_rename(&temp_path, path, contents) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                last_collision = Some(error);
            }
            Err(error) => {
                let _ = std::fs::remove_file(&temp_path);
                return Err(error);
            }
        }
    }
    Err(last_collision.unwrap_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "failed to allocate .env temporary file",
        )
    }))
}

fn write_dotenv_temp_then_rename(
    temp_path: &Path,
    target_path: &Path,
    contents: &str,
) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(temp_path)?;
    set_secret_file_permissions(&file);
    file.write_all(contents.as_bytes())?;
    let _ = file.sync_all();
    drop(file);
    std::fs::rename(temp_path, target_path)?;
    sync_parent_dir(target_path);
    Ok(())
}

fn dotenv_temp_path(path: &Path) -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("env");
    path.with_file_name(format!(".{name}.{}.{}.tmp", std::process::id(), seq))
}

#[cfg(unix)]
fn set_secret_file_permissions(file: &std::fs::File) {
    use std::os::unix::fs::PermissionsExt;
    let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
fn set_secret_file_permissions(_file: &std::fs::File) {}

fn sync_parent_dir(target_path: &Path) {
    let Some(parent) = target_path.parent() else {
        return;
    };
    let Ok(dir) = std::fs::OpenOptions::new().read(true).open(parent) else {
        return;
    };
    let _ = dir.sync_all();
}

pub(super) fn upsert_dotenv_text(original: &str, fields: &[(String, String)]) -> String {
    let mut remaining = fields.to_vec();
    let mut lines = original
        .lines()
        .map(|line| dotenv_line(line, &mut remaining))
        .collect::<Vec<_>>();
    if !remaining.is_empty() && !lines.is_empty() {
        lines.push(String::new());
    }
    lines.extend(
        remaining
            .into_iter()
            .map(|(key, value)| dotenv_assignment(&key, &value)),
    );
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

pub(super) fn remove_dotenv_fields(original: &str, fields: &[String]) -> String {
    let lines = original
        .lines()
        .filter(|line| !dotenv_key(line).is_some_and(|key| fields.iter().any(|field| field == key)))
        .collect::<Vec<_>>();
    if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    }
}

fn dotenv_line(line: &str, remaining: &mut Vec<(String, String)>) -> String {
    let Some(key) = dotenv_key(line) else {
        return line.to_owned();
    };
    let Some(index) = remaining.iter().position(|(field, _)| field == key) else {
        return line.to_owned();
    };
    let (field, value) = remaining.remove(index);
    dotenv_assignment(&field, &value)
}

fn dotenv_key(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') || trimmed.is_empty() {
        return None;
    }
    let (key, _) = trimmed.split_once('=')?;
    let key = key.trim();
    key.chars()
        .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '_')
        .then_some(key)
}

fn dotenv_assignment(key: &str, value: &str) -> String {
    format!("{key}={}", dotenv_value(value))
}

pub(super) fn dotenv_value(value: &str) -> String {
    if value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | ':' | '@'))
    {
        return value.to_owned();
    }
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r");
    format!("\"{escaped}\"")
}
