//! 模拟投资组合 JSON 文件持久化。
//!
//! V1 范围：基础读写 + 原子替换写入（避免崩溃时半写文件）。
//! Redis 后端可后续以 trait 抽象扩展。

use crate::portfolio::SimPortfolio;
use std::io::Write;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PersistError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

pub type PersistResult<T> = Result<T, PersistError>;

/// 将组合序列化为 JSON 字符串。
pub fn to_json(portfolio: &SimPortfolio) -> PersistResult<String> {
    Ok(serde_json::to_string_pretty(portfolio)?)
}

/// 从 JSON 字符串反序列化组合。
pub fn from_json(s: &str) -> PersistResult<SimPortfolio> {
    Ok(serde_json::from_str(s)?)
}

/// 写入到磁盘（原子：先写临时文件再 rename）。
pub fn save_to_file<P: AsRef<Path>>(portfolio: &SimPortfolio, path: P) -> PersistResult<()> {
    let path = path.as_ref();
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;

    let tmp = path.with_extension("tmp");
    {
        let mut file = std::fs::File::create(&tmp)?;
        let s = to_json(portfolio)?;
        file.write_all(s.as_bytes())?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// 从磁盘读取组合。
pub fn load_from_file<P: AsRef<Path>>(path: P) -> PersistResult<SimPortfolio> {
    let s = std::fs::read_to_string(path)?;
    from_json(&s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{OpenRequest, PositionSide};

    fn sample() -> SimPortfolio {
        let mut p = SimPortfolio::new(10_000.0);
        p.open(OpenRequest {
            symbol: "BTC".into(),
            exchange: "binance".into(),
            side: PositionSide::Long,
            quantity: 0.1,
            entry_price: 30_000.0,
            leverage: 1.0,
            fees: 0.0,
            note: "test".into(),
        })
        .unwrap();
        p
    }

    #[test]
    fn json_round_trip_preserves_fields() {
        let p = sample();
        let s = to_json(&p).unwrap();
        let back = from_json(&s).unwrap();
        assert_eq!(p.cash, back.cash);
        assert_eq!(p.position_count(), back.position_count());
        let orig_pos = p.positions.values().next().unwrap();
        let back_pos = back.positions.values().next().unwrap();
        assert_eq!(orig_pos.symbol, back_pos.symbol);
        assert_eq!(orig_pos.quantity, back_pos.quantity);
    }

    #[test]
    fn save_and_load_to_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("portfolio.json");
        let p = sample();
        save_to_file(&p, &path).unwrap();
        assert!(path.exists());

        let loaded = load_from_file(&path).unwrap();
        assert_eq!(loaded.cash, p.cash);
        assert_eq!(loaded.position_count(), 1);
    }

    #[test]
    fn save_overwrites_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("portfolio.json");
        let mut p = sample();
        save_to_file(&p, &path).unwrap();
        // 修改组合后再保存
        p.cash = 999.0;
        save_to_file(&p, &path).unwrap();
        let reloaded = load_from_file(&path).unwrap();
        assert!((reloaded.cash - 999.0).abs() < 1e-9);
    }

    #[test]
    fn load_missing_file_returns_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing.json");
        let result = load_from_file(&path);
        assert!(matches!(result, Err(PersistError::Io(_))));
    }

    #[test]
    fn from_json_with_invalid_payload_errors() {
        let result = from_json("not json");
        assert!(matches!(result, Err(PersistError::Json(_))));
    }

    #[test]
    fn save_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b").join("portfolio.json");
        let p = sample();
        save_to_file(&p, &nested).unwrap();
        assert!(nested.exists());
    }
}
