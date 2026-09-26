//! Validate a complete dotenv document before importing any of its variables.

use crate::{AppError, AppResult};

pub fn parse(contents: &str) -> AppResult<Vec<(String, String)>> {
    let rows = dotenvy::from_read_iter(contents.trim_start_matches('\u{feff}').as_bytes())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| invalid_document())?;
    if rows.iter().any(|(key, value)| key.contains('\0') || value.contains('\0')) {
        return Err(invalid_document());
    }
    Ok(rows)
}

fn invalid_document() -> AppError {
    AppError::Config("环境配置文件格式无效；未加载任何新值，请修复后重启（内容已隐藏）".into())
}

pub(super) fn load() -> AppResult<()> {
    let cwd = std::env::current_dir()
        .map_err(|_| AppError::Config("无法定位环境配置文件；未加载配置".into()))?;
    for directory in cwd.ancestors() {
        let contents = match std::fs::read_to_string(directory.join(".env")) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(_) => return Err(AppError::Config("环境配置文件读取失败；请检查文件权限或路径后重启（内容已隐藏）".into())),
        };
        parse(&contents)?;
        // Reuse dotenvy's environment precedence, substitutions and first-duplicate semantics.
        // Import only after the entire immutable document has passed validation.
        dotenvy::from_read(contents.as_bytes()).map_err(|_| invalid_document())?;
        return Ok(());
    }
    Ok(())
}
