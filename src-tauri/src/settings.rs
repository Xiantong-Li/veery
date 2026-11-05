use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
use winreg::RegKey;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_KEY_CMD: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_NAME: &str = "Veery";
const TASK_NAME: &str = "VeeryStartup";
pub const DEFAULT_SHORTCUT: &str = "Alt+Space";

pub const GITHUB_REPO: &str = "https://github.com/Xiantong-Li/veery";
pub const GITEE_REPO: &str = "https://gitee.com/li-xiantong/veery";

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsInfo {
    pub autostart: bool,
    pub fast_autostart: bool,
    pub shortcut: String,
    pub shortcut_label: String,
    pub language: String,
    pub theme: String,
    pub hint: String,
    pub name: &'static str,
    pub version: &'static str,
    pub author: &'static str,
    pub summary: &'static str,
}

#[derive(Clone)]
pub struct Preferences {
    pub shortcut: String,
    pub language: String,
    pub theme: String,
    pub hint: String,
    pub fast_autostart: bool,
}

#[derive(Serialize, Deserialize)]
struct Saved {
    shortcut: String,
    #[serde(default = "default_language")]
    language: String,
    #[serde(default = "default_theme")]
    theme: String,
    #[serde(default)]
    hint: String,
    #[serde(default)]
    fast_autostart: bool,
}

fn default_language() -> String {
    "zh".to_string()
}

fn default_theme() -> String {
    "light".to_string()
}

pub fn normalize_language(value: &str) -> String {
    if value.eq_ignore_ascii_case("en") {
        "en".to_string()
    } else {
        "zh".to_string()
    }
}

pub fn normalize_theme(value: &str) -> String {
    match value.to_ascii_lowercase().as_str() {
        "dark" => "dark".to_string(),
        "system" => "system".to_string(),
        _ => "light".to_string(),
    }
}

pub fn normalize_hint(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(20)
        .collect()
}

pub fn info(autostart: bool, fast_autostart: bool, prefs: &Preferences) -> SettingsInfo {
    SettingsInfo {
        autostart,
        fast_autostart,
        shortcut_label: shortcut_label(&prefs.shortcut, prefs.language == "en"),
        shortcut: prefs.shortcut.clone(),
        language: prefs.language.clone(),
        theme: prefs.theme.clone(),
        hint: prefs.hint.clone(),
        name: "Veery",
        version: env!("CARGO_PKG_VERSION"),
        author: "Xiantong Li",
        summary: "Veery 是一个轻量的 Windows 启动器。按下全局快捷键弹出搜索框，输入程序的全称、拼音或首字母，回车或点击即可打开开始菜单里的程序。",
    }
}

pub fn shortcut_label(shortcut: &str, english: bool) -> String {
    shortcut
        .split('+')
        .map(|part| match part.trim().to_uppercase().as_str() {
            "ALT" => "Alt".to_string(),
            "CTRL" | "CONTROL" => "Ctrl".to_string(),
            "SHIFT" => "Shift".to_string(),
            "SUPER" => "Win".to_string(),
            "SPACE" => {
                if english {
                    "Space".to_string()
                } else {
                    "空格".to_string()
                }
            }
            "ENTER" => {
                if english {
                    "Enter".to_string()
                } else {
                    "回车".to_string()
                }
            }
            "TAB" => "Tab".to_string(),
            "BACKSPACE" => {
                if english {
                    "Backspace".to_string()
                } else {
                    "退格".to_string()
                }
            }
            "DELETE" => "Delete".to_string(),
            "ESCAPE" | "ESC" => "Esc".to_string(),
            "ARROWUP" | "UP" => {
                if english {
                    "Up".to_string()
                } else {
                    "上".to_string()
                }
            }
            "ARROWDOWN" | "DOWN" => {
                if english {
                    "Down".to_string()
                } else {
                    "下".to_string()
                }
            }
            "ARROWLEFT" | "LEFT" => {
                if english {
                    "Left".to_string()
                } else {
                    "左".to_string()
                }
            }
            "ARROWRIGHT" | "RIGHT" => {
                if english {
                    "Right".to_string()
                } else {
                    "右".to_string()
                }
            }
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

pub fn load(app: &AppHandle) -> Preferences {
    let Some(saved) = config_file(app)
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| serde_json::from_str::<Saved>(&text).ok())
    else {
        return Preferences {
            shortcut: DEFAULT_SHORTCUT.to_string(),
            language: default_language(),
            theme: default_theme(),
            hint: String::new(),
            fast_autostart: false,
        };
    };
    Preferences {
        shortcut: if has_modifier(&saved.shortcut) {
            saved.shortcut
        } else {
            DEFAULT_SHORTCUT.to_string()
        },
        language: normalize_language(&saved.language),
        theme: normalize_theme(&saved.theme),
        hint: normalize_hint(&saved.hint),
        fast_autostart: saved.fast_autostart,
    }
}

pub fn load_shortcut(app: &AppHandle) -> String {
    load(app).shortcut
}

pub fn save_shortcut(app: &AppHandle, shortcut: &str) -> Result<(), String> {
    if !has_modifier(shortcut) {
        return Err("请带上 Ctrl、Alt 或 Shift".to_string());
    }
    let mut prefs = load(app);
    prefs.shortcut = shortcut.to_string();
    store(app, &prefs)
}

pub fn save_hint(app: &AppHandle, hint: &str) -> Result<String, String> {
    let mut prefs = load(app);
    prefs.hint = normalize_hint(hint);
    store(app, &prefs)?;
    Ok(prefs.hint)
}

pub fn save_appearance(app: &AppHandle, language: &str, theme: &str) -> Result<Preferences, String> {
    let mut prefs = load(app);
    prefs.language = normalize_language(language);
    prefs.theme = normalize_theme(theme);
    store(app, &prefs)?;
    Ok(prefs)
}

fn store(app: &AppHandle, prefs: &Preferences) -> Result<(), String> {
    let path = config_file(app)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|error| error.to_string())?;
    }
    let text = serde_json::to_string_pretty(&Saved {
        shortcut: prefs.shortcut.clone(),
        language: prefs.language.clone(),
        theme: prefs.theme.clone(),
        hint: prefs.hint.clone(),
        fast_autostart: prefs.fast_autostart,
    })
    .map_err(|error| error.to_string())?;
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, &text).map_err(|error| error.to_string())?;
    if path.exists() {
        std::fs::remove_file(&path).map_err(|error| error.to_string())?;
    }
    std::fs::rename(&temp, &path).map_err(|error| error.to_string())
}

pub fn open_link(url: &str) -> Result<(), String> {
    const ALLOWED: &[&str] = &[
        GITHUB_REPO,
        GITEE_REPO,
        "https://github.com/Xiantong-Li/veery/releases/latest",
        "https://gitee.com/li-xiantong/veery/releases/latest",
    ];
    if !ALLOWED.contains(&url) {
        return Err("blocked".to_string());
    }
    let wide: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
    let result = unsafe {
        windows::Win32::UI::Shell::ShellExecuteW(
            None,
            windows::core::w!("open"),
            windows::core::PCWSTR(wide.as_ptr()),
            windows::core::PCWSTR(std::ptr::null()),
            windows::core::PCWSTR(std::ptr::null()),
            windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
        )
    };
    if (result.0 as isize) > 32 {
        Ok(())
    } else {
        Err("blocked".to_string())
    }
}

pub fn autostart_enabled() -> bool {
    run_enabled() || task_enabled()
}

pub fn fast_autostart_enabled() -> bool {
    task_enabled()
}

pub fn save_fast_autostart(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let mut prefs = load(app);
    prefs.fast_autostart = enabled;
    store(app, &prefs)
}

pub fn set_autostart(enabled: bool, fast: bool) -> Result<(), String> {
    if !enabled {
        let _ = remove_run();
        let _ = remove_task();
        if autostart_enabled() {
            return Err("开机启动没有写入成功，系统拒绝了这项修改".to_string());
        }
        return Ok(());
    }
    if fast {
        ensure_task()?;
        let _ = remove_run();
        if !task_enabled() {
            return Err("优先加速启动没有写入成功，系统拒绝了这项修改".to_string());
        }
    } else {
        ensure_run()?;
        let _ = remove_task();
        if !run_enabled() {
            return Err("开机启动没有写入成功，系统拒绝了这项修改".to_string());
        }
    }
    Ok(())
}

fn run_enabled() -> bool {
    run_key()
        .ok()
        .and_then(|key| key.get_value::<String, _>(RUN_NAME).ok())
        .is_some()
}

fn task_enabled() -> bool {
    schtasks(&["/Query", "/TN", TASK_NAME]).is_ok()
}

fn ensure_run() -> Result<(), String> {
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
    ])
}

fn remove_run() -> Result<(), String> {
    reg(&["delete", RUN_KEY_CMD, "/v", RUN_NAME, "/f"])
}

fn ensure_task() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    // 登录后立刻启动，不进系统启动项那条延迟队列。只要当前用户，不要管理员。
    schtasks(&[
        "/Create",
        "/TN",
        TASK_NAME,
        "/TR",
        &format!("\"{}\"", exe.display()),
        "/SC",
        "ONLOGON",
        "/IT",
        "/F",
    ])
}

fn remove_task() -> Result<(), String> {
    schtasks(&["/Delete", "/TN", TASK_NAME, "/F"])
}

fn reg(args: &[&str]) -> Result<(), String> {
    run_tool(&reg_exe(), args, "开机启动修改失败")
}

fn schtasks(args: &[&str]) -> Result<(), String> {
    run_tool(&schtasks_exe(), args, "优先加速启动修改失败")
}

fn run_tool(program: &str, args: &[&str], fallback: &str) -> Result<(), String> {
    let output = Command::new(program)
        .args(args)
        .creation_flags(0x08000000)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&output.stderr);
    let detail = detail.trim();
    if detail.is_empty() {
        Err(fallback.to_string())
    } else {
        Err(detail.to_string())
    }
}

fn system32(name: &str) -> String {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    format!(r"{root}\System32\{name}")
}

fn reg_exe() -> String {
    system32("reg.exe")
}

fn schtasks_exe() -> String {
    system32("schtasks.exe")
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
        let was_fast = fast_autostart_enabled();
        set_autostart(true, false).expect("enable");
        assert!(autostart_enabled());
        assert!(!fast_autostart_enabled());
        set_autostart(true, true).expect("enable fast");
        assert!(autostart_enabled());
        assert!(fast_autostart_enabled());
        set_autostart(true, false).expect("disable fast");
        assert!(autostart_enabled());
        assert!(!fast_autostart_enabled());
        set_autostart(false, false).expect("disable");
        assert!(!autostart_enabled());
        assert!(!fast_autostart_enabled());
        if was {
            set_autostart(true, was_fast).expect("restore");
        }
    }
    #[test]
    fn shortcut_requires_a_modifier() {
        assert!(has_modifier("Alt+Space"));
        assert!(has_modifier("Ctrl+Shift+KeyK"));
        assert!(!has_modifier("Space"));
        assert!(!has_modifier(""));
    }

    #[test]
    fn labels_default_shortcut() {
        assert_eq!(shortcut_label("Alt+Space", false), "Alt + 空格");
        assert_eq!(shortcut_label("Ctrl+Shift+KeyK", false), "Ctrl + Shift + K");
        assert_eq!(shortcut_label("Alt+Space", true), "Alt + Space");
    }

    #[test]
    fn old_settings_keep_chinese_and_light() {
        let saved: Saved = serde_json::from_str(r#"{"shortcut":"Alt+Space"}"#).expect("parse");
        assert_eq!(saved.language, "zh");
        assert_eq!(saved.theme, "light");
        assert_eq!(saved.hint, "");
        assert!(!saved.fast_autostart);
        assert_eq!(normalize_hint("  今天想打开什么  "), "今天想打开什么");
        assert_eq!(normalize_hint("123456789012345678901"), "12345678901234567890");
        assert_eq!(normalize_hint("a\nb"), "ab");
        assert_eq!(normalize_language("EN"), "en");
        assert_eq!(normalize_theme("system"), "system");
        assert_eq!(normalize_theme("nope"), "light");
    }
}
