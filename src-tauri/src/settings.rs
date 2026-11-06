use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
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
    // 普通开机启动始终写 Run，和以前一样稳。优先加速是额外的登录任务，不能取代 Run，
    // 否则任务写坏时用户会完全自启不了。
    ensure_run()?;
    if !run_enabled() {
        return Err("开机启动没有写入成功，系统拒绝了这项修改".to_string());
    }
    if fast {
        ensure_task()?;
        if !task_enabled() {
            let _ = remove_task();
            return Err("优先加速启动没有写入成功，系统拒绝了这项修改".to_string());
        }
    } else {
        let _ = remove_task();
        if task_enabled() {
            return Err("旧的优先启动项没有清掉，请再关一次优先加速启动".to_string());
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
    // 注册表里的启动命令需要自带引号，路径有空格时才不会被拆开。
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

fn ensure_task() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|error| error.to_string())?;
    // 先删掉可能写坏的旧任务（错误加引号时，登录会经 cmd 闪黑框）。
    let _ = remove_task();
    // 用 XML 导入，避免 /TR 在不同系统语言、路径空格下被拆坏或包成 cmd。
    create_task_from_xml(&exe)?;
    if !task_enabled() {
        let _ = remove_task();
        return Err("优先加速启动没有写入成功，系统拒绝了这项修改".to_string());
    }
    if !task_action_is_safe(&exe) {
        let _ = remove_task();
        return Err("优先加速启动写入异常，已取消，以免开机弹出黑窗口".to_string());
    }
    Ok(())
}

fn create_task_from_xml(exe: &Path) -> Result<(), String> {
    let xml = task_definition_xml(exe);
    let file = std::env::temp_dir().join(format!("veery-startup-{}.xml", std::process::id()));
    write_utf16_le_file(&file, &xml)?;
    let xml_path = file.to_string_lossy().into_owned();
    let result = schtasks(&["/Create", "/TN", TASK_NAME, "/XML", &xml_path, "/F"]);
    let _ = std::fs::remove_file(&file);
    result
}

fn task_definition_xml(exe: &Path) -> String {
    let command = xml_escape(&exe.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <Enabled>true</Enabled>
    <Hidden>false</Hidden>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{command}</Command>
    </Exec>
  </Actions>
</Task>
"#
    )
}

fn write_utf16_le_file(path: &Path, text: &str) -> Result<(), String> {
    let mut bytes = Vec::with_capacity(2 + text.len() * 2);
    bytes.extend_from_slice(&[0xFF, 0xFE]);
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    std::fs::write(path, bytes).map_err(|error| format!("优先加速启动临时文件写入失败：{error}"))
}

fn xml_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn xml_unescape(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&gt;", ">")
        .replace("&lt;", "<")
        .replace("&amp;", "&")
}

fn remove_task() -> Result<(), String> {
    match schtasks(&["/Delete", "/TN", TASK_NAME, "/F"]) {
        Ok(()) => Ok(()),
        Err(_) if !task_enabled() => Ok(()),
        Err(error) => Err(error),
    }
}

fn remove_run() -> Result<(), String> {
    match reg(&["delete", RUN_KEY_CMD, "/v", RUN_NAME, "/f"]) {
        Ok(()) => Ok(()),
        Err(_) if !run_enabled() => Ok(()),
        Err(error) => Err(error),
    }
}

/// 确认计划任务直接跑本程序，而不是 cmd/powershell（否则登录必闪黑框）。
fn task_action_is_safe(exe: &Path) -> bool {
    let Some(action) = task_action_command() else {
        return false;
    };
    let normalized = action.trim().trim_matches('"').trim();
    let lower = normalized.to_ascii_lowercase();
    if lower.contains("cmd.exe")
        || lower.contains("powershell")
        || lower.contains("pwsh.exe")
        || lower.contains("wscript")
        || lower.contains("cscript")
        || lower.contains("/c ")
    {
        return false;
    }
    let action_path = Path::new(normalized);
    let is_exe = action_path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("exe"));
    is_exe && action_path.file_name() == exe.file_name()
}

fn task_action_command() -> Option<String> {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // XML 不受系统显示语言影响，比 LIST 文案更稳。
    let output = Command::new(schtasks_exe())
        .args(["/Query", "/TN", TASK_NAME, "/XML"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = decode_windows_bytes(&output.stdout);
    extract_xml_command(&text)
}

fn extract_xml_command(text: &str) -> Option<String> {
    let lower = text.to_ascii_lowercase();
    let start_token = "<command>";
    let end_token = "</command>";
    let start = lower.find(start_token)? + start_token.len();
    let end = lower[start..].find(end_token)? + start;
    let raw = text[start..end].trim();
    if raw.is_empty() {
        return None;
    }
    Some(xml_unescape(raw))
}

fn reg(args: &[&str]) -> Result<(), String> {
    run_tool(&reg_exe(), args, "开机启动修改失败")
}

fn schtasks(args: &[&str]) -> Result<(), String> {
    run_tool(&schtasks_exe(), args, "优先加速启动修改失败")
}

fn run_tool(program: &str, args: &[&str], fallback: &str) -> Result<(), String> {
    // CREATE_NO_WINDOW：改设置时绝不能闪出 reg/schtasks 黑框。
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let output = Command::new(program)
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|error| error.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    // schtasks/reg 在中文系统上常回 GBK，按 UTF-8 读会变成一串 �；有的错误在 stdout。
    let detail = {
        let err = decode_windows_bytes(&output.stderr);
        if !err.is_empty() {
            err
        } else {
            decode_windows_bytes(&output.stdout)
        }
    };
    if detail.is_empty() {
        Err(fallback.to_string())
    } else {
        Err(format!("{fallback}：{detail}"))
    }
}

fn decode_windows_bytes(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    // schtasks /XML 有时是 UTF-16 LE（带或不带 BOM）。
    if bytes.starts_with(&[0xFF, 0xFE])
        || (bytes.len() >= 4 && bytes[0] != 0 && bytes[1] == 0 && bytes[2] != 0 && bytes[3] == 0)
    {
        let raw = if bytes.starts_with(&[0xFF, 0xFE]) {
            &bytes[2..]
        } else {
            bytes
        };
        let wide: Vec<u16> = raw
            .chunks_exact(2)
            .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
            .collect();
        return String::from_utf16_lossy(&wide).trim().to_string();
    }
    if let Ok(text) = std::str::from_utf8(bytes) {
        return text.trim().to_string();
    }
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
    fn decodes_utf16_xml_and_extracts_command() {
        let xml = "<?xml version=\"1.0\"?><Task><Actions><Exec><Command>C:\\Apps\\veery.exe</Command></Exec></Actions></Task>";
        let mut bytes = vec![0xFF, 0xFE];
        for unit in xml.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        let text = decode_windows_bytes(&bytes);
        assert_eq!(
            extract_xml_command(&text).as_deref(),
            Some(r"C:\Apps\veery.exe")
        );
        assert_eq!(
            extract_xml_command("<COMMAND>D:\\a&amp;b\\veery.exe</COMMAND>").as_deref(),
            Some(r"D:\a&b\veery.exe")
        );
        let definition = task_definition_xml(Path::new(r"D:\App Gallery\veery.exe"));
        assert!(definition.contains(r"<Command>D:\App Gallery\veery.exe</Command>"));
        assert!(!definition.contains("/TR"));
    }

    #[test]
    fn autostart_round_trip() {
        // 测试进程是 veery_lib-*.exe，不能写回用户原来的开机项，否则登录会启动测试二进制并可能弹控制台。
        let _ = set_autostart(false, false);
        set_autostart(true, false).expect("enable");
        assert!(run_enabled());
        assert!(!fast_autostart_enabled());
        set_autostart(true, true).expect("enable fast");
        assert!(run_enabled());
        assert!(fast_autostart_enabled());
        let exe = std::env::current_exe().expect("exe");
        assert!(task_action_is_safe(&exe), "task must run veery directly, not cmd");
        let action = task_action_command().expect("task action");
        let lower = action.to_ascii_lowercase();
        assert!(!lower.contains("cmd.exe"), "{action}");
        assert!(!lower.contains("powershell"), "{action}");
        assert!(!action.trim_start().starts_with("\"\""), "over-quoted: {action}");
        set_autostart(true, false).expect("disable fast");
        assert!(run_enabled());
        assert!(!fast_autostart_enabled());
        set_autostart(false, false).expect("disable");
        assert!(!autostart_enabled());
        assert!(!fast_autostart_enabled());
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
