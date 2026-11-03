use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use pinyin::ToPinyin;
use serde::Serialize;

pub struct AppEntry {
    pub id: u32,
    pub name: String,
    pub folded: String,
    pub pinyin: String,
    pub initials: String,
    pub path: PathBuf,
}

#[derive(Serialize)]
pub struct SearchHit {
    pub id: u32,
    pub name: String,
    pub icon: Option<String>,
    #[serde(skip)]
    pub path: PathBuf,
}

pub fn scan() -> Vec<AppEntry> {
    let mut by_name = HashMap::new();
    for dir in start_menu_dirs() {
        collect(&dir, &mut by_name);
    }

    let mut apps: Vec<AppEntry> = by_name.into_values().collect();
    apps.sort_by(|a, b| a.folded.cmp(&b.folded));
    for (id, app) in apps.iter_mut().enumerate() {
        app.id = id as u32;
    }
    apps
}

pub fn search(apps: &[AppEntry], query: &str, limit: usize) -> Vec<SearchHit> {
    let query = fold(query);
    if query.is_empty() || limit == 0 {
        return Vec::new();
    }

    let mut scored = Vec::new();
    for app in apps {
        if let Some(value) = score(app, &query) {
            scored.push((value, app));
        }
    }
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.folded.cmp(&b.1.folded)));
    scored.into_iter().take(limit).map(|(_, app)| hit(app)).collect()
}

pub fn open_link(path: &Path) -> Result<(), String> {
    shell_execute(path, "open", "打不开这个程序")
}

pub fn open_elevated(path: &Path) -> Result<(), String> {
    // 用户在 UAC 上点取消时，ShellExecute 返回 5。这不是失败。
    shell_execute(path, "runas", "无法以管理员身份运行").or_else(|error| {
        if error.ends_with("(5)") {
            Ok(())
        } else {
            Err(error)
        }
    })
}

pub fn reveal(path: &Path) -> Result<(), String> {
    let target = link_target(path).filter(|target| !target.as_os_str().is_empty());
    let target = target
        .filter(|target| target.exists())
        .unwrap_or_else(|| path.to_path_buf());
    let mut select = std::ffi::OsString::from("/select,");
    select.push(&target);
    std::process::Command::new("explorer")
        .arg(select)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn shell_execute(path: &Path, verb: &str, failure: &str) -> Result<(), String> {
    use windows::core::PCWSTR;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let wide = wide_path(path);
    let verb = wide_str(verb);
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    let code = result.0 as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(format!("{failure} ({code})"))
    }
}

fn link_target(path: &Path) -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows::core::{Interface, PCWSTR};
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, IPersistFile,
        STGM_READ,
    };
    use windows::Win32::UI::Shell::{IShellLinkW, ShellLink, SLR_NO_UI};

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER).ok()?;
        let store: IPersistFile = link.cast().ok()?;
        let wide = wide_path(path);
        store.Load(PCWSTR(wide.as_ptr()), STGM_READ).ok()?;
        let _ = link.Resolve(HWND(std::ptr::null_mut()), SLR_NO_UI.0 as u32);
        let mut buffer = vec![0u16; 32768];
        link.GetPath(&mut buffer, std::ptr::null_mut(), 0).ok()?;
        let end = buffer.iter().position(|unit| *unit == 0).unwrap_or(0);
        if end == 0 {
            return None;
        }
        Some(PathBuf::from(std::ffi::OsString::from_wide(&buffer[..end])))
    }
}

fn wide_path(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain(std::iter::once(0)).collect()
}

fn wide_str(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn start_menu_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os("ProgramData") {
        dirs.push(PathBuf::from(dir).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    if let Some(dir) = std::env::var_os("APPDATA") {
        dirs.push(PathBuf::from(dir).join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    dirs
}

fn collect(dir: &Path, into: &mut HashMap<String, AppEntry>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, into);
            continue;
        }
        let is_link = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"));
        if !is_link {
            continue;
        }
        let Some(name) = path.file_stem().map(|stem| stem.to_string_lossy().trim().to_string()) else {
            continue;
        };
        if name.is_empty() || is_uninstaller(&name) {
            continue;
        }
        let (folded, pinyin, initials) = index_fields(&name);
        into.insert(
            name.to_lowercase(),
            AppEntry {
                id: 0,
                name,
                folded,
                pinyin,
                initials,
                path,
            },
        );
    }
}

fn is_uninstaller(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("uninstall") || name.contains("卸载")
}

fn index_fields(name: &str) -> (String, String, String) {
    let folded = fold(name);
    let mut word_initials = String::new();
    for word in name.split(|ch: char| !ch.is_ascii_alphanumeric()) {
        if let Some(ch) = word.chars().next() {
            word_initials.push(ch.to_ascii_lowercase());
        }
    }
    if name.is_ascii() {
        return (folded, String::new(), word_initials);
    }

    let mut pinyin_full = String::new();
    let mut pinyin_initials = String::new();
    for ch in name.chars() {
        if let Some(item) = ch.to_pinyin() {
            let plain = item.plain();
            pinyin_full.push_str(plain);
            if let Some(initial) = plain.chars().next() {
                pinyin_initials.push(initial);
            }
        }
    }

    let initials = if pinyin_initials.is_empty() {
        word_initials
    } else if word_initials.is_empty() {
        pinyin_initials
    } else {
        format!("{pinyin_initials}{word_initials}")
    };
    (folded, pinyin_full, initials)
}

fn fold(value: &str) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

fn score(app: &AppEntry, query: &str) -> Option<i32> {
    if app.folded == query || app.pinyin == query {
        return Some(1000);
    }
    if app.folded.starts_with(query) || (!app.pinyin.is_empty() && app.pinyin.starts_with(query)) {
        return Some(800);
    }
    if app.initials == query {
        return Some(780);
    }
    if app.initials.starts_with(query) {
        return Some(700);
    }
    if app.folded.contains(query) || (!app.pinyin.is_empty() && app.pinyin.contains(query)) {
        return Some(500);
    }
    if is_subsequence(query, &app.folded)
        || (!app.pinyin.is_empty() && is_subsequence(query, &app.pinyin))
        || is_subsequence(query, &app.initials)
    {
        return Some(200);
    }
    None
}

fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut rest = haystack.chars();
    needle.chars().all(|ch| rest.any(|candidate| candidate == ch))
}

fn hit(app: &AppEntry) -> SearchHit {
    SearchHit {
        id: app.id,
        name: app.name.clone(),
        icon: None,
        path: app.path.clone(),
    }
}

pub fn icon_data_url(path: &Path) -> Option<String> {
    let png = file_icon_png(path)?;
    let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, png);
    Some(format!("data:image/png;base64,{encoded}"))
}

fn file_icon_png(path: &Path) -> Option<Vec<u8>> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_ICON};
    use windows::Win32::UI::WindowsAndMessaging::DestroyIcon;

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut info = SHFILEINFOW::default();
    let found = unsafe {
        SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_ICON,
        )
    };
    if found == 0 || info.hIcon.is_invalid() {
        return None;
    }
    let png = unsafe { hicon_to_png(info.hIcon) };
    unsafe {
        let _ = DestroyIcon(info.hIcon);
    }
    png
}

unsafe fn hicon_to_png(icon: windows::Win32::UI::WindowsAndMessaging::HICON) -> Option<Vec<u8>> {
    use windows::Win32::Graphics::Gdi::{
        CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
        BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
    };
    use windows::Win32::UI::WindowsAndMessaging::{DrawIconEx, DI_NORMAL};

    const SIZE: i32 = 32;
    let screen = unsafe { GetDC(None) };
    if screen.is_invalid() {
        return None;
    }
    let memory = unsafe { CreateCompatibleDC(Some(screen)) };
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: SIZE,
            biHeight: -SIZE,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
    let bitmap = unsafe {
        CreateDIBSection(Some(screen), &info, DIB_RGB_COLORS, &mut bits, None, 0).ok()
    };
    let Some(bitmap) = bitmap else {
        unsafe {
            let _ = DeleteDC(memory);
            ReleaseDC(None, screen);
        }
        return None;
    };
    unsafe {
        SelectObject(memory, bitmap.into());
        if DrawIconEx(memory, 0, 0, icon, SIZE, SIZE, 0, None, DI_NORMAL).is_err() {
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(memory);
            ReleaseDC(None, screen);
            return None;
        }
    }
    if bits.is_null() {
        unsafe {
            let _ = DeleteObject(bitmap.into());
            let _ = DeleteDC(memory);
            ReleaseDC(None, screen);
        }
        return None;
    }
    let byte_len = (SIZE * SIZE * 4) as usize;
    let raw = unsafe { std::slice::from_raw_parts(bits as *const u8, byte_len) };
    let mut pixels = raw.to_vec();
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        if pixel[3] == 0 && (pixel[0] | pixel[1] | pixel[2]) != 0 {
            pixel[3] = 255;
        }
    }
    unsafe {
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
    }

    let mut output = Vec::new();
    let mut encoder = png::Encoder::new(&mut output, SIZE as u32, SIZE as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().ok()?;
    writer.write_image_data(&pixels).ok()?;
    drop(writer);
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str) -> AppEntry {
        let (folded, pinyin, initials) = index_fields(name);
        AppEntry {
            id: 0,
            name: name.to_string(),
            folded,
            pinyin,
            initials,
            path: PathBuf::new(),
        }
    }

    #[test]
    fn matches_chinese_name_pinyin_and_initials() {
        let app = entry("微信");
        assert!(score(&app, "微信").unwrap() >= 1000);
        assert!(score(&app, "weixin").unwrap() >= 800);
        assert!(score(&app, "wx").unwrap() >= 700);
    }

    #[test]
    fn matches_english_words_and_initials() {
        let app = entry("Visual Studio Code");
        assert!(score(&app, "vsc").unwrap() >= 700);
        assert!(score(&app, "code").unwrap() >= 500);
        assert!(score(&app, "visual").unwrap() >= 800);
    }

    #[test]
    fn start_menu_shortcut_has_png_icon() {
        let Some(app) = scan().into_iter().next() else {
            return;
        };
        let icon = icon_data_url(&app.path).unwrap_or_else(|| panic!("没有图标: {}", app.name));
        assert!(icon.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn shortcut_resolves_to_a_target() {
        let Some(app) = scan().into_iter().next() else {
            return;
        };
        let target = link_target(&app.path).unwrap_or_else(|| panic!("没有目标: {}", app.name));
        assert!(!target.as_os_str().is_empty());
    }
}
