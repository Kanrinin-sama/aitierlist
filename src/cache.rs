use crate::types::CacheState;
use anyhow::{Context, Result};
use serde_json::Value;
use std::path::PathBuf;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const BAKED_SNAPSHOT: &[u8] = include_bytes!("../assets/aa-snapshot.json");

pub fn disk_cache_path() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("locating current executable")?;
    let dir = exe.parent().context("locating executable directory")?;
    Ok(dir.join("aa-cache.json"))
}

pub fn baked_payloads() -> Result<Value> {
    serde_json::from_slice(BAKED_SNAPSHOT).context("parsing baked aa-snapshot.json")
}

pub fn load_payloads() -> Result<(Value, CacheState)> {
    let baked_val = baked_payloads()?;
    let baked_time = baked_val
        .get("fetchedAt")
        .and_then(Value::as_str)
        .and_then(|stamp| OffsetDateTime::parse(stamp, &Rfc3339).ok());

    if let Ok(disk_path) = disk_cache_path()
        && let Ok(bytes) = std::fs::read(&disk_path)
        && let Ok(disk_val) = serde_json::from_slice::<Value>(&bytes)
        && disk_val["agents"].is_array()
        && disk_val["evaluation"]["models"].is_array()
        && disk_val["catalog"].is_array()
    {
        let disk_time = disk_val
            .get("fetchedAt")
            .and_then(Value::as_str)
            .and_then(|stamp| OffsetDateTime::parse(stamp, &Rfc3339).ok());
        if disk_time.is_some_and(|disk| baked_time.is_none_or(|baked| disk >= baked)) {
            return Ok((disk_val, CacheState::Disk));
        }
    }

    Ok((baked_val, CacheState::Baked))
}

pub fn save(payloads: &Value) -> Result<()> {
    let path = disk_cache_path()?;
    let dir = path.parent().context("locating cache directory")?;
    let temp_path = dir.join(format!("aa-cache.tmp.{}", std::process::id()));
    let data = serde_json::to_vec_pretty(payloads).context("serializing cache payloads")?;
    std::fs::write(&temp_path, data)
        .with_context(|| format!("writing temporary cache to {}", temp_path.display()))?;
    if let Err(error) = std::fs::rename(&temp_path, &path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error)
            .with_context(|| format!("renaming {} to {}", temp_path.display(), path.display()));
    }
    Ok(())
}

pub fn fetch_live() -> Result<(Value, CacheState)> {
    let (cached, _) = load_payloads()?;
    let payloads = crate::aa::fetch_payloads(&cached)?;
    let rows = crate::aa::rows_from_payloads(&payloads)?;
    if rows.is_empty() {
        anyhow::bail!("live payload produced zero rows");
    }
    let _ = save(&payloads);
    Ok((payloads, CacheState::Live))
}
