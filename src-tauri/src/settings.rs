use std::path::PathBuf;
use std::process::Command;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_KEY_CMD: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_NAME: &str = "Veery";
pub const DEFAULT_SHORTCUT: &str = "Alt+Space";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsInfo {
    pub autostart: bool,
    pub shortcut: String,
    pub shortcut_label: String,
    pub name: &'static str,
    pub version: &'static str,
    pub author: &'static str,
    pub summary: &'static str,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    shortcut: String,
}

pub fn info(autostart: bool, shortcut: &str) -> SettingsInfo {
    SettingsInfo {
        autostart,
        shortcut_label: shortcut_label(shortcut),
        shortcut: shortcut.to_string(),
        name: "Veery",
        version: env!("CARGO_PKG_VERSION"),
        author: "Xiantong Li",
        summary: "Veery 是一个轻量的 Windows 启动器。按下全局快捷键弹出搜索框，输入程序的全称、拼音或首字母，回车或点击即可打开开始菜单里的程序。",
    }
}

pub fn shortcut_label(shortcut: &str) -> String {
    shortcut
        .split('+')
        .map(|part| match part.trim().to_uppercase().as_str() {
            "ALT" => "Alt".to_string(),
            "CTRL" | "CONTROL" => "Ctrl".to_string(),
            "SHIFT" => "Shift".to_string(),
            "SUPER" => "Win".to_string(),
            "SPACE" => "空格".to_string(),
            "ENTER" => "回车".to_string(),
            "TAB" => "Tab".to_string(),
            "BACKSPACE" => "退格".to_string(),
            "DELETE" => "Delete".to_string(),
            "ESCAPE" | "ESC" => "Esc".to_string(),
            "ARROWUP" | "UP" => "上".to_string(),
            "ARROWDOWN" | "DOWN" => "下".to_string(),
            "ARROWLEFT" | "LEFT" => "左".to_string(),
            "ARROWRIGHT" | "RIGHT" => "右".to_string(),
            other if other.starts_with("KEY") && other.len() == 4 => other[3..].to_string(),
            other if other.starts_with("DIGIT") => other[5..].to_string(),
            other => {
                let mut chars = other.chars();
                match chars.next() {
                    Some(first) => first.to_string() + chars.as_str(),
                    None => String::new(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" + ")
}

pub fn has_modifier(shortcut: &str) -> bool {
    let upper = shortcut.to_uppercase();
    upper.contains("ALT")
        || upper.contains("CTRL")
        || upper.contains("CONTROL")
        || upper.contains("SHIFT")
        || upper.contains("SUPER")
}

pub fn load_shortcut(app: &AppHandle) -> String {
    let Ok(path) = config_file(app) else {
        return DEFAULT_SHORTCUT.to_string();
    };
    let Ok(text) = std::fs::read_to_string(path) else {
        return DEFAULT_SHORTCUT.to_string();
    };
    serde_json::from_str::<Saved>(&text)
        .ok()
        .map(|saved| saved.shortcut)
        .filter(|shortcut| has_modifier(shortcut))
        .unwrap_or_else(|| DEFAULT_SHORTCUT.to_string())
}

pub fn save_shortcut(app: &AppHandle, shortcut: &str) -> Result<(), String> {
    let path = config_file(app)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    }
    let text = serde_json::to_string_pretty(&Saved {
        shortcut: shortcut.to_string(),
    })
    .map_err(|error| error.to_string())?;
    std::fs::write(path, text).map_err(|error| error.to_string())
}

pub fn autostart_enabled() -> bool {
    run_key()
        .ok()
        .and_then(|key| key.get_value::<String, _>(RUN_NAME).ok())
        .is_some()
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    if enabled {
        let exe = std::env::current_exe().map_err(|error| error.to_string())?;
        reg(&[
            "add",
            RUN_KEY_CMD,
            "/v",
            RUN_NAME,
            "/t",
            "REG_SZ",
            "/d",
            &format!("\"{}\"", exe.display()),
            "/f",
        ])?;
    } else {
        let _ = reg(&["delete", RUN_KEY_CMD, "/v", RUN_NAME, "/f"]);
    }
    if autostart_enabled() != enabled {
        return Err("开机启动没有写入成功，系统拒绝了这项修改".to_string());
    }
    Ok(())
}

fn reg(args: &[&str]) -> Result<(), String> {
    let output = Command::new(reg_exe())
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr);
    let detail = detail.trim();
    if detail.is_empty() {
        Err("开机启动修改失败".to_string())
    } else {
        Err(detail.to_string())
    }
}

fn reg_exe() -> String {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    format!(r"{root}\System32\reg.exe")
}

fn run_key() -> Result<RegKey, String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    hkcu.open_subkey_with_flags(RUN_KEY, KEY_READ | KEY_WRITE)
        .or_else(|_| hkcu.create_subkey(RUN_KEY).map(|(key, _)| key))
        .map_err(|error| error.to_string())
}

fn config_file(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?
        .join("settings.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autostart_round_trip() {
        let was = autostart_enabled();
        set_autostart(true).expect("enable");
        assert!(autostart_enabled());
        set_autostart(false).expect("disable");
        assert!(!autostart_enabled());
        if was {
            set_autostart(true).expect("restore");
        }
    }
    #[test]
    fn labels_default_shortcut() {
        assert_eq!(shortcut_label("Alt+Space"), "Alt + 空格");
        assert_eq!(shortcut_label("Ctrl+Shift+KeyK"), "Ctrl + Shift + K");
    }
}
