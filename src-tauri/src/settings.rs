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
const SERIALIZE_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\Serialize";
const STARTUP_DELAY_VALUE: &str = "StartupDelayInMSec";
/// 0.2.6/0.2.7 用过的登录计划任务；引号写坏时会经 cmd 拉起并弹黑框。只删不建。
const LEGACY_TASK_NAME: &str = "VeeryStartup";
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
    run_enabled()
}

pub fn save_fast_autostart(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let mut prefs = load(app);
    prefs.fast_autostart = enabled;
    store(app, &prefs)
}

/// 只写 / 删 HKCU Run。绝不碰启动延迟——那是另一个开关。
pub fn set_autostart(enabled: bool) -> Result<(), String> {
    if !enabled {
        let _ = remove_run();
        if run_enabled() {
            return Err("开机启动没有写入成功，系统拒绝了这项修改".to_string());
        }
        return Ok(());
    }
    ensure_run()?;
    if !run_enabled() {
        return Err("开机启动没有写入成功，系统拒绝了这项修改".to_string());
    }
    Ok(())
}

/// 只改 HKCU 启动延迟。绝不写 Run——那是另一个开关。
pub fn apply_fast_autostart(fast: bool) -> Result<(), String> {
    if fast {
        ensure_startup_delay_disabled()?;
        if !startup_delay_disabled() {
            return Err("优先加速启动没有写入成功，系统拒绝了这项修改".to_string());
        }
    } else {
        clear_startup_delay()?;
        if startup_delay_disabled() {
            return Err("旧的优先启动项没有清掉，请再关一次优先加速启动".to_string());
        }
    }
    Ok(())
}

/// 后台静默删除旧版 VeeryStartup。只删一次、不 Query，避免挡界面；CREATE_NO_WINDOW 避免再闪框。
pub fn spawn_legacy_task_cleanup() {
    std::thread::spawn(|| {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let _ = Command::new(system32("schtasks.exe"))
            .args(["/Delete", "/TN", LEGACY_TASK_NAME, "/F"])
            .creation_flags(CREATE_NO_WINDOW)
            .output();
    });
}

fn run_enabled() -> bool {
    run_key()
        .ok()
        .and_then(|key| key.get_value::<String, _>(RUN_NAME).ok())
        .is_some()
}

fn startup_delay_disabled() -> bool {
    serialize_key()
        .ok()
        .and_then(|key| key.get_value::<u32, _>(STARTUP_DELAY_VALUE).ok())
        .is_some_and(|value| value == 0)
}

fn ensure_run() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    let command = run_command(&exe);
    if !run_command_is_safe(&command) {
        return Err("开机启动命令不安全，已拒绝写入".to_string());
    }
    // 注册表里的启动命令需要自带引号，路径有空格时才不会被拆开；直接指向 exe，绝不套 cmd。
    reg(&[
        "add",
        RUN_KEY_CMD,
        "/v",
        RUN_NAME,
        "/t",
        "REG_SZ",
        "/d",
        &command,
        "/f",
    ])?;
    let written = run_command_value().unwrap_or_default();
    if !run_command_is_safe(&written) {
        let _ = remove_run();
        return Err("开机启动写入后校验失败，已撤回".to_string());
    }
    Ok(())
}

fn run_command(exe: &std::path::Path) -> String {
    format!("\"{}\"", exe.display())
}

fn run_command_value() -> Option<String> {
    run_key()
        .ok()
        .and_then(|key| key.get_value::<String, _>(RUN_NAME).ok())
}

fn run_command_is_safe(command: &str) -> bool {
    let trimmed = command.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    // 旧版曾用 cmd/powershell 包一层，登录必闪黑框。
    if lower.contains("cmd.exe")
        || lower.contains("powershell")
        || lower.contains("pwsh.exe")
        || lower.contains("conhost")
    {
        return false;
    }
    // 允许 "C:\path\veery.exe"，拒绝奇怪的多段命令。
    let path = trimmed.trim_matches('"');
    let path_lower = path.to_ascii_lowercase();
    path_lower.ends_with(".exe") && !path.contains('&') && !path.contains('|') && !path.contains('>')
}

fn ensure_startup_delay_disabled() -> Result<(), String> {
    let key = serialize_key()?;
    key.set_value(STARTUP_DELAY_VALUE, &0u32)
        .map_err(|error| format!("优先加速启动修改失败：{error}"))
}

fn clear_startup_delay() -> Result<(), String> {
    let Ok(key) = serialize_key() else {
        return Ok(());
    };
    match key.delete_value(STARTUP_DELAY_VALUE) {
        Ok(()) => Ok(()),
        Err(_) if !startup_delay_disabled() => Ok(()),
        Err(error) => Err(format!("优先加速启动修改失败：{error}")),
    }
}

fn remove_run() -> Result<(), String> {
    match reg(&["delete", RUN_KEY_CMD, "/v", RUN_NAME, "/f"]) {
        Ok(()) => Ok(()),
        Err(_) if !run_enabled() => Ok(()),
        Err(error) => Err(error),
    }
}

fn serialize_key() -> Result<RegKey, String> {
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    hkcu.create_subkey(SERIALIZE_KEY)
        .map(|(key, _)| key)
        .map_err(|error| error.to_string())
}

fn reg(args: &[&str]) -> Result<(), String> {
    // CREATE_NO_WINDOW：改设置时绝不能闪出 reg 黑框。
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let output = Command::new(reg_exe())
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    // reg 在中文系统上常回 GBK，按 UTF-8 读会变成一串 �；有的错误在 stdout。
    let detail = {
        let err = decode_windows_bytes(&output.stderr);
        if !err.is_empty() {
            err
        } else {
            decode_windows_bytes(&output.stdout)
        }
    };
    if detail.is_empty() {
        Err("开机启动修改失败".to_string())
    } else {
        Err(format!("开机启动修改失败：{detail}"))
    }
}

fn decode_windows_bytes(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.trim().to_string();
    }
    // 中文 Windows 控制台错误多半是系统 ANSI 代码页（简体即 GBK）。
    decode_acp(bytes)
}

fn decode_acp(bytes: &[u8]) -> String {
    use windows::Win32::Globalization::{MultiByteToWideChar, CP_ACP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS};

    unsafe {
        let needed = MultiByteToWideChar(CP_ACP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, None);
        if needed <= 0 {
            return String::from_utf8_lossy(bytes).trim().to_string();
        }
        let mut wide = vec![0u16; needed as usize];
        let written = MultiByteToWideChar(CP_ACP, MULTI_BYTE_TO_WIDE_CHAR_FLAGS(0), bytes, Some(&mut wide));
        if written <= 0 {
            return String::from_utf8_lossy(bytes).trim().to_string();
        }
        String::from_utf16_lossy(&wide[..written as usize]).trim().to_string()
    }
}

fn system32(name: &str) -> String {
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    format!(r"{root}\System32\{name}")
}

fn reg_exe() -> String {
    system32("reg.exe")
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
    fn decodes_gbk_console_errors() {
        // “错误: 拒绝访问。” 的 GBK 字节；按 UTF-8 会读成替换符。
        let gbk = [
            0xB4u8, 0xED, 0xCE, 0xF3, 0x3A, 0x20, 0xBE, 0xDC, 0xBE, 0xF8, 0xB7, 0xC3, 0xCE, 0xCA, 0xA1, 0xA3,
        ];
        let text = decode_windows_bytes(&gbk);
        assert!(text.contains('错') || text.contains("拒绝") || text.contains("访问"), "{text}");
        assert!(!text.contains('\u{FFFD}'), "{text}");
    }

    #[test]
    fn autostart_round_trip() {
        // 测试进程是 veery_lib-*.exe，不能写回用户原来的开机项，否则登录会启动测试二进制并可能弹控制台。
        let _ = apply_fast_autostart(false);
        let _ = set_autostart(false);
        set_autostart(true).expect("enable");
        assert!(run_enabled());
        let command = run_command_value().expect("run value");
        assert!(run_command_is_safe(&command), "unsafe run command: {command}");
        assert!(!startup_delay_disabled(), "opening Run must not enable fast");
        apply_fast_autostart(true).expect("enable fast");
        assert!(run_enabled());
        assert!(startup_delay_disabled());
        apply_fast_autostart(false).expect("disable fast");
        assert!(run_enabled());
        assert!(!startup_delay_disabled());
        set_autostart(false).expect("disable");
        assert!(!autostart_enabled());
    }

    #[test]
    fn run_command_rejects_console_hosts() {
        assert!(run_command_is_safe(r#""C:\Program Files\Veery\veery.exe""#));
        assert!(!run_command_is_safe(r#"cmd.exe /c "C:\Veery\veery.exe""#));
        assert!(!run_command_is_safe(
            r#"powershell -WindowStyle Hidden -Command Start-Process 'C:\Veery\veery.exe'"#
        ));
        assert!(!run_command_is_safe(r#""C:\Veery\veery.exe" & pause"#));
    }

    #[test]
    fn enabling_autostart_does_not_touch_startup_delay() {
        let _ = apply_fast_autostart(false);
        let _ = set_autostart(false);
        assert!(!startup_delay_disabled());
        set_autostart(true).expect("enable run only");
        assert!(run_enabled());
        assert!(!startup_delay_disabled(), "two toggles must stay independent");
        set_autostart(false).expect("cleanup");
    }

    #[test]
    fn fast_autostart_toggle_stays_under_200ms() {
        // 优先加速只能碰 HKCU 注册表；一旦又拖进外部进程，这个门槛会立刻打脸。
        let _ = apply_fast_autostart(false);
        let _ = set_autostart(false);
        set_autostart(true).expect("enable run");

        let started = std::time::Instant::now();
        apply_fast_autostart(true).expect("fast on");
        let on = started.elapsed();
        assert!(startup_delay_disabled());
        assert!(
            on < std::time::Duration::from_millis(200),
            "enable took {on:?}, path must not shell out"
        );

        let started = std::time::Instant::now();
        apply_fast_autostart(false).expect("fast off");
        let off = started.elapsed();
        assert!(!startup_delay_disabled());
        assert!(
            off < std::time::Duration::from_millis(200),
            "disable took {off:?}, path must not shell out"
        );

        set_autostart(false).expect("cleanup");
        eprintln!("fast toggle on={on:?} off={off:?}");
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
