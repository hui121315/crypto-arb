use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use super::CredentialUpdateError;

const DOTENV_TEMP_ATTEMPTS: usize = 5;

pub(super) fn apply_fields_to_dotenv(
    updates: &[(String, String)],
    clears: &[String],
) -> Result<(), CredentialUpdateError> {
    #[cfg(not(test))]
    let path = env_file_path()?;
    #[cfg(test)]
    let path = {
        let Some(root) = std::env::var_os("CROSSLINE_SETTINGS_BROWSER_DIR") else { return Ok(()) };
        let root = PathBuf::from(root).canonicalize().map_err(CredentialUpdateError::Persist)?;
        let temp = std::env::temp_dir().canonicalize().map_err(CredentialUpdateError::Persist)?;
        if !root.starts_with(temp) || !root.join("isolated-settings-fixture").is_file() {
            return Err(CredentialUpdateError::SecretBackend("invalid isolated storage directory".into()));
        }
        root.join(".env")
    };
    apply_fields_to_dotenv_path(&path, updates, clears)
}

pub(super) fn apply_fields_to_dotenv_path(
    path: &Path,
    updates: &[(String, String)],
    clears: &[String],
) -> Result<(), CredentialUpdateError> {
    let original = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => return Err(CredentialUpdateError::Persist(error)),
    };
    let updated = rewrite_dotenv_text(&original, updates, clears)?;
    if updated == original { return Ok(()); }
    write_dotenv_atomic(path, &updated).map_err(CredentialUpdateError::Persist)
}

#[cfg(test)]
pub(super) fn persist_fields_to_dotenv(
    fields: &[(String, String)],
) -> Result<(), CredentialUpdateError> {
    apply_fields_to_dotenv(fields, &[])
}

#[cfg(test)]
pub(super) fn persist_fields_to_dotenv_path(
    path: &Path,
    fields: &[(String, String)],
) -> Result<(), CredentialUpdateError> {
    apply_fields_to_dotenv_path(path, fields, &[])
}

#[cfg(test)]
pub(super) fn remove_fields_from_dotenv_path(
    path: &Path,
    fields: &[String],
) -> Result<(), CredentialUpdateError> {
    apply_fields_to_dotenv_path(path, &[], fields)
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
    set_secret_file_permissions(&file)?;
    file.write_all(contents.as_bytes())?;
    file.sync_all()?;
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
fn set_secret_file_permissions(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_secret_file_permissions(_file: &std::fs::File) -> std::io::Result<()> { Ok(()) }

fn sync_parent_dir(target_path: &Path) {
    let Some(parent) = target_path.parent() else {
        return;
    };
    let Ok(dir) = std::fs::OpenOptions::new().read(true).open(parent) else {
        return;
    };
    let _ = dir.sync_all();
}

#[cfg(test)]
pub(super) fn upsert_dotenv_text(original: &str, fields: &[(String, String)]) -> Result<String, CredentialUpdateError> {
    rewrite_dotenv_text(original, fields, &[])
}

#[cfg(test)]
pub(super) fn remove_dotenv_fields(original: &str, fields: &[String]) -> Result<String, CredentialUpdateError> {
    rewrite_dotenv_text(original, &[], fields)
}

fn rewrite_dotenv_text(original: &str, updates: &[(String, String)], clears: &[String]) -> Result<String, CredentialUpdateError> {
    use common::config::env_file::parse;
    let invalid = || CredentialUpdateError::SecretBackend(
        "环境配置文件格式无效或无法完整保存；原文件与当前凭证未更改，请修复配置后重试（内容已隐藏）".into());
    parse(original).map_err(|_| invalid())?;
    let mut remaining = updates.to_vec();
    let mut text = String::new();
    let mut record = String::new();
    // Let dotenvy's parser find logical records, including export and multiline values.
    // Keep unrelated records verbatim; replace every occurrence of a changed key.
    for line in original.split_inclusive('\n') {
        record.push_str(line);
        let Ok(rows) = parse(&record) else { continue };
        if rows.len() > 1 { return Err(invalid()); }
        let key = rows.first().map(|(key, _)| key);
        if let Some(key) = key.filter(|key| clears.contains(key) || updates.iter().any(|(name, _)| name == *key)) {
            if !clears.contains(key) {
                if let Some(index) = remaining.iter().position(|(name, _)| name == key) {
                    let (name, value) = remaining.remove(index);
                    text.push_str(&dotenv_assignment(&name, &value));
                    text.push('\n');
                }
            }
        } else {
            text.push_str(&record);
        }
        record.clear();
    }
    if !record.is_empty() { return Err(invalid()); }
    for (key, value) in remaining.iter().filter(|(key, _)| !clears.contains(key)) {
        if !text.is_empty() && !text.ends_with('\n') { text.push('\n'); }
        text.push_str(&dotenv_assignment(key, value));
        text.push('\n');
    }
    let rows = parse(&text).map_err(|_| invalid())?;
    for key in clears {
        if rows.iter().any(|(name, _)| name == key) { return Err(invalid()); }
    }
    for (key, value) in updates.iter().filter(|(key, _)| !clears.contains(key)) {
        let matches: Vec<_> = rows.iter().filter(|(name, _)| name == key).collect();
        if matches.len() != 1 || matches[0].1 != *value { return Err(invalid()); }
    }
    Ok(text)
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
        .replace('$', "\\$")
        .replace('\n', "\\n");
    format!("\"{escaped}\"")
}
