use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const FILE_NAME: &str = "launch_history.json";
const MAX_QUERIES: usize = 200;
const MAX_PATHS_PER_QUERY: usize = 12;

#[derive(Clone, Serialize, Deserialize)]
struct Entry {
    path: String,
    count: u32,
    last: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct Saved {
    /// folded query → launches
    queries: HashMap<String, Vec<Entry>>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn fold_query(query: &str) -> String {
    query
        .chars()
        .flat_map(|ch| ch.to_lowercase())
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn history_path(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?
        .join(FILE_NAME))
}

fn load(app: &AppHandle) -> Saved {
    history_path(app)
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn store(app: &AppHandle, saved: &Saved) -> Result<(), String> {
    let path = history_path(app)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    }
    let text = serde_json::to_string_pretty(saved).map_err(|error| error.to_string())?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, &text).map_err(|error| error.to_string())?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|error| error.to_string())?;
    }
    std::fs::rename(&temp, &path).map_err(|error| error.to_string())
}

/// 同一查询下的启动偏好：path → (加分, 最近使用时间)。
/// 每次搜索只读一遍历史文件，评分循环里不再碰磁盘。
pub fn boosts(app: &AppHandle, query: &str) -> HashMap<String, (i32, u64)> {
    let key = fold_query(query);
    if key.is_empty() {
        return HashMap::new();
    }
    let saved = load(app);
    let Some(entries) = saved.queries.get(&key) else {
        return HashMap::new();
    };
    entries
        .iter()
        .map(|entry| (entry.path.clone(), (tier(entry.count), entry.last)))
        .collect()
}

fn tier(count: u32) -> i32 {
    match count {
        0 => 0,
        1 => 150,
        2 => 250,
        3 => 320,
        _ => 400,
    }
}

pub fn record(app: &AppHandle, query: &str, path: &Path) -> Result<(), String> {
    let key = fold_query(query);
    if key.is_empty() {
        return Ok(());
    }
    let path_key = path.to_string_lossy().into_owned();
    let mut saved = load(app);
    let entries = saved.queries.entry(key.clone()).or_default();
    if let Some(entry) = entries.iter_mut().find(|entry| entry.path == path_key) {
        entry.count = entry.count.saturating_add(1);
        entry.last = now_secs();
    } else {
        entries.push(Entry {
            path: path_key,
            count: 1,
            last: now_secs(),
        });
    }
    entries.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| right.last.cmp(&left.last))
    });
    entries.truncate(MAX_PATHS_PER_QUERY);
    if saved.queries.len() > MAX_QUERIES {
        let mut keys: Vec<_> = saved.queries.keys().cloned().collect();
        keys.sort();
        while saved.queries.len() > MAX_QUERIES {
            if let Some(old) = keys.pop() {
                saved.queries.remove(&old);
            } else {
                break;
            }
        }
    }
    store(app, &saved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fold_query_trims_and_lowers() {
        assert_eq!(fold_query("  G  "), "g");
        assert_eq!(fold_query("Foo Bar"), "foo bar");
    }

    #[test]
    fn boost_tiers_rise_with_usage() {
        assert_eq!(tier(0), 0);
        assert_eq!(tier(1), 150);
        assert_eq!(tier(2), 250);
        assert_eq!(tier(3), 320);
        assert_eq!(tier(9), 400);
    }
}
