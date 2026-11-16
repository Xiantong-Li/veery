use base64::Engine;
use std::collections::HashMap;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

const HOST: &str = "image.200228.top";
const POSES: [&str; 6] = ["idle", "shrug", "dizzy", "point", "happy", "tap"];
const MAX_FILE: usize = 2 * 1024 * 1024;

pub fn manifest() -> Result<String, String> {
    let (status, body) =
        crate::net::get(HOST, "/veery/manifest.json", 262_144).map_err(|_| "offline")?;
    // 还没上传清单时安静地当作没有在线伙伴，不算错误。
    if status == 404 {
        return Err("empty".to_string());
    }
    if status != 200 {
        return Err("failed".to_string());
    }
    String::from_utf8(body).map_err(|_| "bad manifest".to_string())
}

/// 只允许下载 mascots/ 下的角色图片，避免把命令当成任意 URL 请求器。
pub fn file(path: &str) -> Result<String, String> {
    let allowed = path.starts_with("mascots/")
        && path.ends_with(".png")
        && path.len() <= 128
        && !path.contains("..")
        && path
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'/' | b'.'));
    if !allowed {
        return Err("blocked".to_string());
    }
    let (status, body) =
        crate::net::get(HOST, &format!("/veery/{path}"), MAX_FILE).map_err(|_| "offline")?;
    if status != 200 {
        return Err("offline".to_string());
    }
    Ok(base64::engine::general_purpose::STANDARD.encode(body))
}

fn cache_root(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?
        .join("mascots"))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 24
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub fn save_pose(app: &AppHandle, id: &str, pose: &str, data: &str) -> Result<(), String> {
    if !valid_id(id) || !POSES.contains(&pose) {
        return Err("blocked".to_string());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|error| error.to_string())?;
    if bytes.is_empty() || bytes.len() > MAX_FILE {
        return Err("bad file".to_string());
    }
    let dir = cache_root(app)?.join(id);
    std::fs::create_dir_all(&dir).map_err(|error| error.to_string())?;
    let path = dir.join(format!("{pose}.png"));
    let temp = dir.join(format!("{pose}.png.tmp"));
    std::fs::write(&temp, &bytes).map_err(|error| error.to_string())?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|error| error.to_string())?;
    }
    std::fs::rename(&temp, &path).map_err(|error| error.to_string())
}

/// 返回已缓存的姿势（base64），可能不全；缺哪个由前端补下载。
pub fn read_cached(app: &AppHandle, id: &str) -> Result<HashMap<String, String>, String> {
    let mut poses = HashMap::new();
    if !valid_id(id) {
        return Ok(poses);
    }
    let dir = cache_root(app)?.join(id);
    for pose in POSES {
        if let Ok(bytes) = std::fs::read(dir.join(format!("{pose}.png"))) {
            poses.insert(pose.to_string(), base64::engine::general_purpose::STANDARD.encode(bytes));
        }
    }
    Ok(poses)
}

pub fn cached_ids(app: &AppHandle) -> Vec<String> {
    let Ok(root) = cache_root(app) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !valid_id(&name) {
            continue;
        }
        let dir = entry.path();
        if POSES.iter().all(|pose| dir.join(format!("{pose}.png")).is_file()) {
            ids.push(name);
        }
    }
    ids.sort();
    ids
}