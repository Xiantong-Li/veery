use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use pinyin::ToPinyin;
use serde::Serialize;

pub struct AppEntry {
    pub id: u32,
    pub name: String,
    pub alias: String,
    pub folded: String,
    pub pinyin: String,
    pub syllables: String,
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

pub fn scan() -> Result<Vec<AppEntry>, String> {
    let dirs = app_dirs();
    if dirs.is_empty() {
        return Err("找不到快捷方式目录".to_string());
    }
    let mut saw_dir = false;
    let mut by_name = HashMap::new();
    let mut seen = HashSet::new();
    for (dir, depth) in dirs {
        if !dir.exists() {
            continue;
        }
        saw_dir = true;
        collect(&dir, &mut by_name, &mut seen, 0, depth);
    }
    // 计算器、终端这类应用没有快捷方式，只登记在系统的应用列表里。
    collect_shell_apps(&mut by_name);
    if !saw_dir {
        return Err("快捷方式目录不存在".to_string());
    }

    let mut apps: Vec<AppEntry> = by_name.into_values().collect();
    apps.sort_by(|left, right| left.folded.cmp(&right.folded).then_with(|| left.path.cmp(&right.path)));
    for (id, app) in apps.iter_mut().enumerate() {
        app.id = id as u32;
    }
    Ok(apps)
}

pub fn search(apps: &[AppEntry], query: &str, limit: usize) -> Vec<SearchHit> {
    search_with_boost(apps, query, limit, |_, _| (0, 0))
}

/// 只列快捷方式的路径、修改时间和大小，不解快捷方式、不取图标。
/// 唤出时拿它和上次的值比，几毫秒就能判断有没有新装或卸载的程序。
pub fn fingerprint() -> Option<u64> {
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut saw_dir = false;
    for (dir, depth) in app_dirs() {
        if !dir.exists() {
            continue;
        }
        saw_dir = true;
        collect_links(&dir, &mut entries, &mut seen, 0, depth);
    }
    if !saw_dir {
        return None;
    }
    Some(fingerprint_of(entries))
}

fn collect_links(
    dir: &Path,
    into: &mut Vec<(String, u64, u64)>,
    seen: &mut HashSet<PathBuf>,
    depth: u8,
    limit: u8,
) {
    if depth > limit {
        return;
    }
    let identity = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    if !seen.insert(identity) {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            collect_links(&path, into, seen, depth + 1, limit);
            continue;
        }
        let is_link = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"));
        if !is_link {
            continue;
        }
        // Windows 上 DirEntry::metadata 直接复用目录枚举的数据，不额外读盘。
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let modified = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
            .map(|elapsed| elapsed.as_nanos() as u64)
            .unwrap_or(0);
        into.push((path.to_string_lossy().to_string(), modified, meta.len()));
    }
}

fn fingerprint_of(mut entries: Vec<(String, u64, u64)>) -> u64 {
    entries.sort();
    let mut hasher = DefaultHasher::new();
    for (path, modified, len) in entries {
        path.hash(&mut hasher);
        modified.hash(&mut hasher);
        len.hash(&mut hasher);
    }
    hasher.finish()
}

/// `boost` 返回 (加分, 最近使用时间)。同一查询下选过的程序会排得更靠前。
pub fn search_with_boost(
    apps: &[AppEntry],
    query: &str,
    limit: usize,
    boost: impl Fn(&str, &Path) -> (i32, u64),
) -> Vec<SearchHit> {
    let tokens: Vec<String> = query.split_whitespace().map(fold).filter(|token| !token.is_empty()).collect();
    if tokens.is_empty() || limit == 0 {
        return Vec::new();
    }

    let mut best: Vec<(i32, u64, &AppEntry)> = Vec::with_capacity(limit);
    for app in apps {
        let Some(base) = score_query(app, &tokens) else {
            continue;
        };
        let (extra, last) = boost(query, &app.path);
        let value = base.saturating_add(extra);
        let ahead = best.iter().position(|(kept_score, kept_last, kept)| {
            value > *kept_score
                || (value == *kept_score && last > *kept_last)
                || (value == *kept_score && last == *kept_last && app.folded < kept.folded)
                || (value == *kept_score
                    && last == *kept_last
                    && app.folded == kept.folded
                    && app.id < kept.id)
        });
        if let Some(index) = ahead {
            best.insert(index, (value, last, app));
            if best.len() > limit {
                best.pop();
            }
        } else if best.len() < limit {
            best.push((value, last, app));
        }
    }
    best.into_iter().map(|(_, _, app)| hit(app)).collect()
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
    if is_shell_app(path) {
        return Err("这个程序没有文件位置".to_string());
    }
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
        // 线程可能已经初始化过 COM。失败时仍尝试创建 ShellLink，已有的套间可以继续用。
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

/// 每个目录配一个允许往下翻的层数。桌面放在开始菜单前面，同名时以开始菜单的为准。
fn app_dirs() -> Vec<(PathBuf, u8)> {
    let mut dirs: Vec<(PathBuf, u8)> = desktop_dirs().into_iter().map(|dir| (dir, 0)).collect();
    if let Some(dir) = std::env::var_os("ProgramData") {
        dirs.push((PathBuf::from(dir).join(r"Microsoft\Windows\Start Menu\Programs"), 12));
    }
    if let Some(dir) = std::env::var_os("APPDATA") {
        dirs.push((PathBuf::from(dir).join(r"Microsoft\Windows\Start Menu\Programs"), 12));
    }
    dirs
}

/// 有些程序只往桌面放快捷方式，开始菜单里没有；桌面只取顶层，不跟着进用户的文件夹。
fn desktop_dirs() -> Vec<PathBuf> {
    use windows::Win32::UI::Shell::{
        SHGetKnownFolderPath, FOLDERID_Desktop, FOLDERID_PublicDesktop, KF_FLAG_DEFAULT,
    };

    [FOLDERID_Desktop, FOLDERID_PublicDesktop]
        .iter()
        .filter_map(|id| shell_string(unsafe { SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None) }))
        .map(PathBuf::from)
        .collect()
}

fn collect(
    dir: &Path,
    into: &mut HashMap<String, AppEntry>,
    seen: &mut HashSet<PathBuf>,
    depth: u8,
    limit: u8,
) {
    if depth > limit {
        return;
    }
    let identity = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    if !seen.insert(identity) {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect(&path, into, seen, depth + 1, limit);
            continue;
        }
        let is_link = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"));
        if !is_link {
            continue;
        }
        let Some(stem) = path.file_stem().map(|stem| stem.to_string_lossy().trim().to_string()) else {
            continue;
        };
        if stem.is_empty() || is_uninstaller(&stem) {
            continue;
        }
        // 快捷方式文件名经常是英文，开始菜单里显示的是本地化名字。
        let shown = display_name(&path).filter(|name| !name.is_empty()).unwrap_or_else(|| stem.clone());
        if is_uninstaller(&shown) {
            continue;
        }
        let app = indexed(&shown, &stem, path);
        into.insert(shown.to_lowercase(), app);
    }
}

fn is_uninstaller(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.contains("uninstall") || name.contains("卸载")
}

fn unresolved_name(name: &str) -> bool {
    name.starts_with("@{") || name.starts_with("ms-resource:")
}

fn is_shell_app(path: &Path) -> bool {
    path.to_string_lossy().starts_with("shell:")
}

fn app_path(parsing: &str) -> Option<PathBuf> {
    let parsing = parsing.trim();
    if parsing.is_empty() {
        return None;
    }
    // AppsFolder 里会混进网页快捷方式，parsing 名就是 http(s) 地址，提不出图标也不该当程序。
    let lower = parsing.to_ascii_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:")
        || lower.starts_with("ms-settings:")
    {
        return None;
    }
    if parsing.contains('\\') || parsing.contains('/') {
        Some(PathBuf::from(parsing))
    } else {
        Some(PathBuf::from(format!(r"shell:AppsFolder\{parsing}")))
    }
}

fn shell_string(value: windows::core::Result<windows::core::PWSTR>) -> Option<String> {
    let ptr = value.ok()?;
    let text = if ptr.is_null() {
        String::new()
    } else {
        let text = unsafe { String::from_utf16_lossy(ptr.as_wide()) };
        unsafe {
            windows::Win32::System::Com::CoTaskMemFree(Some(ptr.as_ptr() as *const core::ffi::c_void));
        }
        text
    };
    let text = text.trim();
    if text.is_empty() { None } else { Some(text.to_string()) }
}

fn collect_shell_apps(into: &mut HashMap<String, AppEntry>) {
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED, IBindCtx};
    use windows::Win32::UI::Shell::{
        IEnumShellItems, IShellItem, SHGetKnownFolderItem, BHID_EnumItems, FOLDERID_AppsFolder, KF_FLAG_DEFAULT,
        SIGDN_NORMALDISPLAY, SIGDN_PARENTRELATIVEPARSING,
    };

    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let Ok(folder) = SHGetKnownFolderItem::<IShellItem>(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None) else {
            return;
        };
        let Ok(items) = folder.BindToHandler::<_, IEnumShellItems>(None::<&IBindCtx>, &BHID_EnumItems) else {
            return;
        };
        for _ in 0..4096 {
            let mut fetched = 0u32;
            let mut batch = [None];
            if items.Next(&mut batch, Some(&mut fetched)).is_err() || fetched == 0 {
                break;
            }
            let Some(item) = batch[0].take() else {
                break;
            };
            let Some(name) = shell_string(item.GetDisplayName(SIGDN_NORMALDISPLAY)) else {
                continue;
            };
            if is_uninstaller(&name) || unresolved_name(&name) {
                continue;
            }
            let key = name.to_lowercase();
            if into.contains_key(&key) {
                continue;
            }
            let Some(parsing) = shell_string(item.GetDisplayName(SIGDN_PARENTRELATIVEPARSING)) else {
                continue;
            };
            let Some(path) = app_path(&parsing) else {
                continue;
            };
            into.insert(key, indexed(&name, &name, path));
        }
    }
}

fn indexed(shown: &str, stem: &str, path: PathBuf) -> AppEntry {
    let (mut folded, mut pinyin, mut syllables, mut initials) = index_fields(shown);
    let alias = if shown.eq_ignore_ascii_case(stem) {
        String::new()
    } else {
        let (alias_folded, alias_pinyin, alias_syllables, alias_initials) = index_fields(stem);
        folded = format!("{folded}\n{alias_folded}");
        if !alias_pinyin.is_empty() {
            pinyin = if pinyin.is_empty() {
                alias_pinyin
            } else {
                format!("{pinyin}\n{alias_pinyin}")
            };
        }
        if !alias_syllables.is_empty() {
            syllables = if syllables.is_empty() {
                alias_syllables
            } else {
                format!("{syllables}\n{alias_syllables}")
            };
        }
        if !alias_initials.is_empty() {
            initials = if initials.is_empty() {
                alias_initials
            } else {
                format!("{initials}\n{alias_initials}")
            };
        }
        stem.to_string()
    };
    AppEntry {
        id: 0,
        name: shown.to_string(),
        alias,
        folded,
        pinyin,
        syllables,
        initials,
        path,
    }
}

fn display_name(path: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES;
    use windows::Win32::UI::Shell::{SHGetFileInfoW, SHFILEINFOW, SHGFI_DISPLAYNAME};

    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut info = SHFILEINFOW::default();
    let found = unsafe {
        SHGetFileInfoW(
            PCWSTR(wide.as_ptr()),
            FILE_FLAGS_AND_ATTRIBUTES(0),
            Some(&mut info),
            std::mem::size_of::<SHFILEINFOW>() as u32,
            SHGFI_DISPLAYNAME,
        )
    };
    if found == 0 {
        return None;
    }
    let end = info.szDisplayName.iter().position(|unit| *unit == 0).unwrap_or(info.szDisplayName.len());
    let name = String::from_utf16_lossy(&info.szDisplayName[..end]).trim().to_string();
    if name.is_empty() { None } else { Some(name) }
}

fn index_fields(name: &str) -> (String, String, String, String) {
    let folded = fold(name);
    let mut word_initials = String::new();
    for word in name.split(|ch: char| !ch.is_ascii_alphanumeric()) {
        if let Some(ch) = word.chars().next() {
            word_initials.push(ch.to_ascii_lowercase());
        }
    }
    if name.is_ascii() {
        return (folded, String::new(), String::new(), word_initials);
    }

    let mut pinyin_full = String::new();
    let mut syllables = String::new();
    let mut pinyin_initials = String::new();
    for ch in name.chars() {
        if let Some(item) = ch.to_pinyin() {
            let plain = item.plain();
            pinyin_full.push_str(plain);
            if !syllables.is_empty() {
                syllables.push(' ');
            }
            syllables.push_str(plain);
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
    (folded, pinyin_full, syllables, initials)
}

fn fold(value: &str) -> String {
    let mut folded = String::with_capacity(value.len());
    for ch in value.chars() {
        if ch.is_whitespace() {
            continue;
        }
        let ch = match ch as u32 {
            code @ 0xFF01..=0xFF5E => char::from_u32(code - 0xFEE0).unwrap_or(ch),
            _ => ch,
        };
        for lower in ch.to_lowercase() {
            folded.push(lower);
        }
    }
    folded
}

fn score_query(app: &AppEntry, tokens: &[String]) -> Option<i32> {
    let mut total: i32 = 0;
    for token in tokens {
        total = total.saturating_add(score(app, token)?);
    }
    Some(total)
}

fn score(app: &AppEntry, query: &str) -> Option<i32> {
    let mut best = 0;
    let mut matched = false;
    let mut keep = |value: i32| {
        matched = true;
        if value > best {
            best = value;
        }
    };
    for part in app.folded.split('\n') {
        if let Some(value) = score_text(part, query) {
            keep(value);
        }
    }
    for part in app.pinyin.split('\n') {
        if let Some(value) = score_text(part, query) {
            keep(value);
        }
    }
    if let Some(value) = score_words(&app.name, query) {
        keep(value);
    }
    if !app.alias.is_empty() {
        if let Some(value) = score_words(&app.alias, query) {
            keep(value);
        }
    }
    for part in app.syllables.split('\n') {
        if let Some(value) = score_syllables(part, query) {
            keep(value);
        }
    }
    for part in app.initials.split('\n') {
        if let Some(value) = score_initials(part, query) {
            keep(value);
        }
    }
    // 「资源文件管理器」和系统显示的「文件资源管理器」是同一组字，只是顺序不同。
    if same_characters(query, primary_name(&app.folded)) {
        keep(500);
    }
    if query.chars().count() >= 5 && query.is_ascii() {
        if let Some(value) = score_typo(app, query) {
            keep(value);
        }
    }
    drop(keep);
    if !matched && query.chars().count() >= 3 && (is_subsequence(query, &app.folded) || (!app.pinyin.is_empty() && is_subsequence(query, &app.pinyin)) || is_subsequence(query, &app.initials))
    {
        matched = true;
        best = best.max(200);
    }
    matched.then_some(best)
}

fn score_text(text: &str, query: &str) -> Option<i32> {
    if text.is_empty() || query.is_empty() {
        return None;
    }
    if text == query {
        Some(1000)
    } else if text.starts_with(query) {
        Some(800)
    } else if text.contains(query) {
        Some(500)
    } else {
        None
    }
}

fn score_words(name: &str, query: &str) -> Option<i32> {
    let mut best = None;
    let mut first = true;
    for word in name.split(|ch: char| !ch.is_ascii_alphanumeric()) {
        let word = fold(word);
        if word.is_empty() {
            continue;
        }
        let value = if word == query {
            if first { 900 } else { 760 }
        } else if word.starts_with(query) {
            if first { 850 } else { 720 }
        } else {
            first = false;
            continue;
        };
        best = Some(best.map_or(value, |current: i32| current.max(value)));
        first = false;
    }
    best
}

fn score_syllables(syllables: &str, query: &str) -> Option<i32> {
    let parts: Vec<&str> = syllables.split_whitespace().filter(|part| !part.is_empty()).collect();
    if parts.is_empty() {
        return None;
    }
    let mut best = None;
    for start in 0..parts.len() {
        let mut joined = String::new();
        for (offset, syllable) in parts[start..].iter().enumerate() {
            if joined.len() + syllable.len() > query.len() {
                break;
            }
            joined.push_str(syllable);
            if joined.len() != query.len() {
                if !query.starts_with(&joined) {
                    break;
                }
                continue;
            }
            if joined != query {
                break;
            }
            let value = if start == 0 && offset + 1 == parts.len() {
                1000
            } else if start == 0 {
                800
            } else {
                860
            };
            best = Some(best.map_or(value, |current: i32| current.max(value)));
            break;
        }
    }
    best
}

fn score_initials(initials: &str, query: &str) -> Option<i32> {
    if initials.is_empty() {
        return None;
    }
    if initials == query {
        Some(780)
    } else if initials.starts_with(query) {
        Some(700)
    } else if query.chars().count() >= 3 && initials.contains(query) {
        Some(660)
    } else {
        None
    }
}

fn score_typo(app: &AppEntry, query: &str) -> Option<i32> {
    let close = app
        .folded
        .split('\n')
        .chain(app.name.split(|ch: char| !ch.is_ascii_alphanumeric()))
        .chain(app.alias.split(|ch: char| !ch.is_ascii_alphanumeric()))
        .map(fold)
        .filter(|text| text.chars().count() >= 5)
        .any(|text| within_one_edit(&text, query));
    close.then_some(420)
}

fn primary_name(folded: &str) -> &str {
    folded.split('\n').next().unwrap_or(folded)
}

fn same_characters(query: &str, name: &str) -> bool {
    let count = query.chars().count();
    if count < 4 || query.chars().all(|ch| ch.is_ascii()) || count != name.chars().count() {
        return false;
    }
    let mut left: Vec<char> = query.chars().collect();
    let mut right: Vec<char> = name.chars().collect();
    left.sort_unstable();
    right.sort_unstable();
    left == right
}

fn is_subsequence(needle: &str, haystack: &str) -> bool {
    let mut rest = haystack.chars();
    needle.chars().all(|ch| rest.any(|candidate| candidate == ch))
}

fn within_one_edit(left: &str, right: &str) -> bool {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    let (left_len, right_len) = (left.len(), right.len());
    if left_len.abs_diff(right_len) > 1 || left == right {
        return false;
    }
    if left_len == right_len {
        let mut mismatches = Vec::new();
        for index in 0..left_len {
            if left[index] != right[index] {
                mismatches.push(index);
            }
        }
        return mismatches.len() == 1
            || (mismatches.len() == 2
                && mismatches[1] == mismatches[0] + 1
                && left[mismatches[0]] == right[mismatches[1]]
                && left[mismatches[1]] == right[mismatches[0]]);
    }
    let (longer, shorter) = if left_len > right_len { (&left, &right) } else { (&right, &left) };
    let mut long_index = 0;
    let mut short_index = 0;
    let mut skipped = false;
    while long_index < longer.len() && short_index < shorter.len() {
        if longer[long_index] == shorter[short_index] {
            long_index += 1;
            short_index += 1;
        } else if skipped {
            return false;
        } else {
            skipped = true;
            long_index += 1;
        }
    }
    true
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
    if png_is_blank(&png) {
        return None;
    }
    let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, png);
    Some(format!("data:image/png;base64,{encoded}"))
}

fn png_is_blank(png: &[u8]) -> bool {
    let Ok(decoder) = png::Decoder::new(std::io::Cursor::new(png)).read_info() else {
        return true;
    };
    let mut reader = decoder;
    let mut buf = vec![0; reader.output_buffer_size()];
    if reader.next_frame(&mut buf).is_err() {
        return true;
    }
    !buf.chunks_exact(4).any(|pixel| pixel[3] > 8 && (pixel[0] | pixel[1] | pixel[2]) > 8)
}

fn file_icon_png(path: &Path) -> Option<Vec<u8>> {
    if is_shell_app(path) {
        return shell_icon_png(path);
    }
    // SHGetFileInfoW 给出的 HICON 由调用方负责销毁，编码失败时也不能漏掉。
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

fn shell_icon_png(path: &Path) -> Option<Vec<u8>> {
    use windows::core::{Interface, PCWSTR};
    use windows::Win32::Foundation::SIZE;
    use windows::Win32::Graphics::Gdi::DeleteObject;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED, IBindCtx};
    use windows::Win32::UI::Shell::{
        IShellItem, IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
    };

    let wide = wide_path(path);
    unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let item = SHCreateItemFromParsingName::<_, _, IShellItem>(PCWSTR(wide.as_ptr()), None::<&IBindCtx>).ok()?;
        let factory: IShellItemImageFactory = item.cast().ok()?;
        // 先要纯图标；部分 Store 应用在 ICONONLY 下会给出全透明图，再退回普通缩略图。
        let flags = [
            SIIGBF(SIIGBF_BIGGERSIZEOK.0 | SIIGBF_ICONONLY.0),
            SIIGBF_BIGGERSIZEOK,
        ];
        for flag in flags {
            let Ok(bitmap) = factory.GetImage(SIZE { cx: 32, cy: 32 }, flag) else {
                continue;
            };
            if bitmap.is_invalid() {
                continue;
            }
            let png = hbitmap_to_png(bitmap);
            let _ = DeleteObject(bitmap.into());
            if let Some(png) = png.filter(|png| !png_is_blank(png)) {
                return Some(png);
            }
        }
        None
    }
}

unsafe fn hbitmap_to_png(bitmap: windows::Win32::Graphics::Gdi::HBITMAP) -> Option<Vec<u8>> {
    use windows::Win32::Graphics::Gdi::{
        GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
    };

    let mut bitmap_info = BITMAP::default();
    let wrote = unsafe {
        GetObjectW(
            HGDIOBJ(bitmap.0),
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bitmap_info as *mut BITMAP as *mut core::ffi::c_void),
        )
    };
    if wrote == 0 || bitmap_info.bmWidth <= 0 || bitmap_info.bmHeight <= 0 {
        return None;
    }
    let width = bitmap_info.bmWidth as u32;
    let height = bitmap_info.bmHeight as u32;
    if width > 256 || height > 256 {
        return None;
    }
    let screen = unsafe { GetDC(None) };
    if screen.is_invalid() {
        return None;
    }
    let mut info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: bitmap_info.bmWidth,
            biHeight: -(bitmap_info.bmHeight),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut pixels = vec![0u8; (width * height * 4) as usize];
    let lines = unsafe {
        GetDIBits(
            screen,
            bitmap,
            0,
            height,
            Some(pixels.as_mut_ptr() as *mut core::ffi::c_void),
            &mut info,
            DIB_RGB_COLORS,
        )
    };
    unsafe {
        ReleaseDC(None, screen);
    }
    if lines <= 0 {
        return None;
    }
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        let alpha = pixel[3];
        if alpha == 0 {
            if pixel[0] | pixel[1] | pixel[2] != 0 {
                pixel[3] = 255;
            }
        } else if alpha < 255 {
            for channel in &mut pixel[..3] {
                *channel = ((*channel as u16 * 255) / alpha as u16).min(255) as u8;
            }
        }
    }
    let mut output = Vec::new();
    let mut encoder = png::Encoder::new(&mut output, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut writer = encoder.write_header().ok()?;
    writer.write_image_data(&pixels).ok()?;
    drop(writer);
    Some(output)
}

unsafe fn hicon_to_png(icon: windows::Win32::UI::WindowsAndMessaging::HICON) -> Option<Vec<u8>> {
    // 屏幕 DC、内存 DC 和位图在每条返回路径上释放。图标句柄由 file_icon_png 销毁。
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
    if memory.is_invalid() {
        unsafe {
            ReleaseDC(None, screen);
        }
        return None;
    }
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
        let (folded, pinyin, syllables, initials) = index_fields(name);
        AppEntry {
            id: 0,
            name: name.to_string(),
            alias: String::new(),
            folded,
            pinyin,
            syllables,
            initials,
            path: PathBuf::new(),
        }
    }

    #[test]
    fn fingerprint_tracks_shortcut_changes() {
        let base = vec![
            (r"C:\menu\a.lnk".to_string(), 10, 100),
            (r"C:\menu\b.lnk".to_string(), 20, 200),
        ];
        let reordered = vec![base[1].clone(), base[0].clone()];
        assert_eq!(fingerprint_of(base.clone()), fingerprint_of(reordered));

        let mut added = base.clone();
        added.push((r"C:\menu\c.lnk".to_string(), 30, 300));
        assert_ne!(fingerprint_of(base.clone()), fingerprint_of(added));

        let mut touched = base.clone();
        touched[0].1 = 11;
        assert_ne!(fingerprint_of(base), fingerprint_of(touched));
    }

    #[test]
    fn start_menu_fingerprint_is_stable() {
        let first = fingerprint().unwrap_or_else(|| panic!("没有指纹"));
        let second = fingerprint().unwrap_or_else(|| panic!("没有指纹"));
        assert_eq!(first, second);
    }

    #[test]
    fn explorer_matches_the_name_windows_shows() {
        let app = indexed("文件资源管理器", "File Explorer", PathBuf::new());
        assert_eq!(app.name, "文件资源管理器");
        assert!(score(&app, "文件资源管理器").unwrap() >= 1000);
        assert!(score(&app, "资源管理器").unwrap() >= 500);
        assert!(score(&app, "资源文件管理器").unwrap() >= 500);
        assert!(score(&app, "explorer").unwrap() >= 760);
        assert!(score(&app, "zyglq").unwrap() >= 660);
        assert!(score(&app, "ziyuan").unwrap() >= 860);
        assert!(score(&app, "explroer").unwrap() >= 420);
    }

    #[test]
    fn several_words_must_all_match() {
        let hits = search(&apps(&["Visual Studio Code", "Visual Studio", "Code"]), "visual code", 8);
        assert_eq!(hits[0].name, "Visual Studio Code");
        assert!(hits.iter().all(|hit| hit.name != "Visual Studio"));
        let hits = search(&apps(&["Visual Studio Code"]), "vs code", 8);
        assert_eq!(hits[0].name, "Visual Studio Code");
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
        let Some(app) = scan().ok().and_then(|apps| apps.into_iter().find(|app| is_shortcut(&app.path))) else {
            return;
        };
        let icon = icon_data_url(&app.path).unwrap_or_else(|| panic!("没有图标: {}", app.name));
        assert!(icon.starts_with("data:image/png;base64,"));
    }

    #[test]
    fn shortcut_resolves_to_a_target() {
        let Some(app) = scan().ok().and_then(|apps| apps.into_iter().find(|app| is_shortcut(&app.path))) else {
            return;
        };
        let target = link_target(&app.path).unwrap_or_else(|| panic!("没有目标: {}", app.name));
        assert!(!target.as_os_str().is_empty());
    }

    fn is_shortcut(path: &Path) -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("lnk"))
    }

    #[test]
    fn app_path_keeps_files_and_wraps_app_ids() {
        assert_eq!(
            app_path(r"Microsoft.WindowsCalculator_8wekyb3d8bbwe!App").unwrap(),
            PathBuf::from(r"shell:AppsFolder\Microsoft.WindowsCalculator_8wekyb3d8bbwe!App")
        );
        assert_eq!(app_path(r"C:\Apps\Tool.exe").unwrap(), PathBuf::from(r"C:\Apps\Tool.exe"));
        assert!(app_path("  ").is_none());
    }

    #[test]
    fn calculator_and_terminal_are_searchable() {
        let Ok(apps) = scan() else {
            return;
        };
        for name in ["计算器", "终端"] {
            let Some(app) = apps.iter().find(|app| app.name == name) else {
                panic!("应用列表里没有{name}");
            };
            let path = app.path.to_string_lossy().to_ascii_lowercase();
            assert!(
                path.starts_with(r"shell:appsfolder\") || path.ends_with(".lnk"),
                "{name} -> {}",
                app.path.display()
            );
            assert_eq!(search(&apps, name, 8).first().map(|hit| hit.name.as_str()), Some(name));
            let icon = icon_data_url(&app.path);
            assert!(
                icon.as_ref().is_some_and(|icon| icon.starts_with("data:image/png;base64,")),
                "{name} 没有图标"
            );
        }
    }

    #[test]
    fn app_path_skips_web_links() {
        assert!(app_path("https://github.com/git-for-windows/git/wiki/FAQ").is_none());
        assert!(app_path("http://localhost:8080/workspace").is_none());
        assert!(app_path("mailto:someone@example.com").is_none());
    }

    fn apps(names: &[&str]) -> Vec<AppEntry> {
        names
            .iter()
            .enumerate()
            .map(|(id, name)| {
                let mut app = entry(name);
                app.id = id as u32;
                app
            })
            .collect()
    }

    #[test]
    fn empty_and_blank_queries_return_nothing() {
        let apps = apps(&["记事本"]);
        assert!(search(&apps, "", 8).is_empty());
        assert!(search(&apps, "   ", 8).is_empty());
        assert!(search(&apps, "记事本", 0).is_empty());
    }

    #[test]
    fn english_exact_and_prefix() {
        let apps = apps(&["Notepad"]);
        assert_eq!(search(&apps, "Notepad", 8)[0].name, "Notepad");
        assert_eq!(search(&apps, "note", 8)[0].name, "Notepad");
    }

    #[test]
    fn chinese_full_pinyin_and_initials() {
        let apps = apps(&["微信"]);
        assert_eq!(search(&apps, "微信", 8)[0].name, "微信");
        assert_eq!(search(&apps, "weixin", 8)[0].name, "微信");
        assert_eq!(search(&apps, "wx", 8)[0].name, "微信");
    }

    #[test]
    fn mixed_name_matches_letters_and_pinyin() {
        let apps = apps(&["QQ音乐"]);
        let app = &apps[0];
        assert!(
            search(&apps, "qq", 8).iter().any(|hit| hit.name == "QQ音乐"),
            "folded={} pinyin={} initials={}",
            app.folded,
            app.pinyin,
            app.initials
        );
        assert!(
            app.pinyin.contains("yin"),
            "folded={} pinyin={} initials={}",
            app.folded,
            app.pinyin,
            app.initials
        );
        assert!(search(&apps, "yin", 8).iter().any(|hit| hit.name == "QQ音乐"));
    }

    #[test]
    fn contains_and_subsequence_match() {
        let apps = apps(&["Visual Studio Code"]);
        assert_eq!(search(&apps, "studio", 8)[0].name, "Visual Studio Code");
        assert_eq!(search(&apps, "vscd", 8)[0].name, "Visual Studio Code");
    }

    #[test]
    fn exact_name_ranks_ahead_of_a_longer_name() {
        let hits = search(&apps(&["Visual Studio Code", "Code"]), "code", 8);
        assert_eq!(hits[0].name, "Code");
        assert_eq!(hits[1].name, "Visual Studio Code");
    }

    #[test]
    fn leading_word_ranks_ahead_of_later_word_prefix() {
        let hits = search(&apps(&["BPS Work Client", "Clash Verge"]), "cl", 8);
        assert_eq!(hits[0].name, "Clash Verge");
        assert_eq!(hits[1].name, "BPS Work Client");
    }

    #[test]
    fn preference_boost_can_reorder_same_tier_hits() {
        let mut list = apps(&["Gimp", "Google Chrome"]);
        list[0].path = PathBuf::from("C:\\apps\\gimp.exe");
        list[1].path = PathBuf::from("C:\\apps\\chrome.exe");
        let chrome = PathBuf::from("C:\\apps\\chrome.exe");
        let hits = search_with_boost(&list, "g", 8, |_query, path| {
            if path == &chrome {
                (400, 9)
            } else {
                (0, 0)
            }
        });
        assert_eq!(hits[0].name, "Google Chrome");
    }

    #[test]
    fn limit_keeps_the_stable_name_order() {
        let hits = search(&apps(&["Beta Note", "Alpha Note", "Alpine Note"]), "note", 2);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].name, "Alpha Note");
        assert_eq!(hits[1].name, "Alpine Note");
    }

    #[test]
    fn later_directory_replaces_duplicate_name() {
        let root = std::env::temp_dir().join(format!("veery-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let first = root.join("common");
        let second = root.join("user");
        fs::create_dir_all(&first).unwrap();
        fs::create_dir_all(&second).unwrap();
        fs::write(first.join("记事本.lnk"), b"").unwrap();
        fs::write(second.join("记事本.lnk"), b"").unwrap();
        let mut into = HashMap::new();
        let mut seen = HashSet::new();
        collect(&first, &mut into, &mut seen, 0, 12);
        collect(&second, &mut into, &mut seen, 0, 12);
        let app = into.get("记事本").unwrap();
        assert!(app.path.starts_with(&second));
        collect(&second, &mut into, &mut seen, 0, 12);
        assert_eq!(into.len(), 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn depth_limit_skips_subdirectories() {
        let root = std::env::temp_dir().join(format!("veery-depth-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let nested = root.join("folder");
        fs::create_dir_all(&nested).unwrap();
        fs::write(root.join("oss-browser.lnk"), b"").unwrap();
        fs::write(nested.join("buried.lnk"), b"").unwrap();
        let mut into = HashMap::new();
        let mut seen = HashSet::new();
        collect(&root, &mut into, &mut seen, 0, 0);
        assert_eq!(into.len(), 1);
        assert!(into.contains_key("oss-browser"));
        let mut links = Vec::new();
        let mut seen = HashSet::new();
        collect_links(&root, &mut links, &mut seen, 0, 0);
        assert_eq!(links.len(), 1);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn search_core_stays_within_a_few_milliseconds() {
        let mut apps = Vec::new();
        for id in 0..300 {
            let mut app = entry(&format!("Program {id}"));
            app.id = id;
            apps.push(app);
        }
        let start = std::time::Instant::now();
        for _ in 0..30 {
            let _ = search(&apps, "pro", 8);
        }
        let average = start.elapsed() / 30;
        eprintln!("veery search core average: {average:?}");
        assert!(average.as_millis() < 20, "{average:?}");
    }
}
