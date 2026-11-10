mod catalog;
mod files;
mod history;
mod settings;
mod update;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WindowEvent, Wry};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{EnumChildWindows, SC_KEYMENU, WM_SYSCOMMAND};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

struct IconCache {
    ready: HashMap<PathBuf, String>,
}

/// 文件搜索默认关闭。开启后只在桌面/文档/下载里建一份轻量列表，
/// 打开开关或超过刷新间隔才重新扫描，搜索本身不碰磁盘。
struct FileState {
    enabled: AtomicBool,
    types: Mutex<Vec<String>>,
    entries: Mutex<Vec<files::FileEntry>>,
    ready: AtomicBool,
    scanning: AtomicBool,
    scanned_at: Mutex<Option<Instant>>,
    hits: Mutex<HashMap<u32, PathBuf>>,
    icons: Mutex<HashMap<PathBuf, Option<String>>>,
    next_id: AtomicU32,
}

const FILE_SCAN_STALE: Duration = Duration::from_secs(600);
const FILE_HITS_CAP: usize = 3000;
const FILE_ICONS_CAP: usize = 1024;

struct TrayItems {
    show: MenuItem<Wry>,
    settings: MenuItem<Wry>,
    shortcut: MenuItem<Wry>,
    quit: MenuItem<Wry>,
}

struct AppState {
    index: Mutex<Vec<catalog::AppEntry>>,
    index_ready: AtomicBool,
    index_scanning: AtomicBool,
    index_checking: AtomicBool,
    index_fingerprint: Mutex<Option<u64>>,
    icons: Mutex<IconCache>,
    files: FileState,
    shown_at: Mutex<Instant>,
    blur_until: Mutex<Instant>,
    settings_open: AtomicBool,
    last_query: Mutex<String>,
    shortcut: Mutex<String>,
    tray: Mutex<Option<TrayItems>>,
}

fn mark_shown(app: &AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut shown_at) = state.shown_at.lock() {
            *shown_at = Instant::now();
        }
    }
}

fn mark_blur_guard(app: &AppHandle) {
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut until) = state.blur_until.lock() {
            *until = Instant::now() + Duration::from_millis(800);
        }
    }
}

fn set_memory_level(window: &tauri::WebviewWindow, low: bool) {
    let _ = window.with_webview(move |webview| unsafe {
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            ICoreWebView2_3, ICoreWebView2_19, COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW,
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL,
        };
        use windows::core::Interface;

        let Ok(core) = webview.controller().CoreWebView2() else {
            return;
        };
        if !low {
            if let Ok(core) = core.cast::<ICoreWebView2_3>() {
                let _ = core.Resume();
            }
        }
        let Ok(core) = core.cast::<ICoreWebView2_19>() else {
            return;
        };
        let level = if low {
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_LOW
        } else {
            COREWEBVIEW2_MEMORY_USAGE_TARGET_LEVEL_NORMAL
        };
        let _ = core.SetMemoryUsageTargetLevel(level);
    });
}

fn held(vk: i32) -> bool {
    unsafe { (GetAsyncKeyState(vk) as u16) & 0x8000 != 0 }
}

unsafe extern "system" fn suppress_system_menu(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _data: usize,
) -> LRESULT {
    if msg == WM_SYSCOMMAND && (wparam.0 & 0xFFF0) == SC_KEYMENU as usize {
        return LRESULT(0);
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

unsafe extern "system" fn subclass_child(hwnd: HWND, _lparam: LPARAM) -> windows::core::BOOL {
    unsafe {
        let _ = SetWindowSubclass(hwnd, Some(suppress_system_menu), 1, 0);
    }
    windows::core::BOOL(1)
}

fn guard_alt_space(window: &tauri::WebviewWindow) {
    // 录入快捷键时拦住主窗口的 Alt+空格系统菜单，不改其他程序的快捷键。
    if let Ok(hwnd) = window.hwnd() {
        unsafe {
            let _ = SetWindowSubclass(hwnd, Some(suppress_system_menu), 1, 0);
            let _ = EnumChildWindows(Some(hwnd), Some(subclass_child), LPARAM(0));
        }
    }
    let window = window.clone();
    let _ = window.clone().with_webview(move |webview| unsafe {
        use webview2_com::AcceleratorKeyPressedEventHandler;
        use webview2_com::Microsoft::Web::WebView2::Win32::{
            COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN, COREWEBVIEW2_KEY_EVENT_KIND_KEY_UP,
            COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN, COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_UP,
        };

        let handler = AcceleratorKeyPressedEventHandler::create(Box::new(move |_sender, args| {
            let Some(args) = args else {
                return Ok(());
            };
            let mut kind = COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN;
            let mut key = 0u32;
            args.KeyEventKind(&mut kind)?;
            args.VirtualKey(&mut key)?;
            if key != VK_SPACE as u32 || !held(VK_MENU) {
                return Ok(());
            }
            let down = kind == COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN
                || kind == COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN;
            let up = kind == COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_UP
                || kind == COREWEBVIEW2_KEY_EVENT_KIND_KEY_UP;
            if !down && !up {
                return Ok(());
            }
            args.SetHandled(true)?;
            if down && !ALT_SPACE_DOWN.swap(true, Ordering::Relaxed) {
                let _ = window.emit(
                    "alt-space",
                    HeldKeys {
                        ctrl: held(VK_CONTROL),
                        shift: held(VK_SHIFT),
                        alt: true,
                    },
                );
            }
            if up {
                ALT_SPACE_DOWN.store(false, Ordering::Relaxed);
            }
            Ok(())
        }));
        let mut token = 0i64;
        let _ = webview.controller().add_AcceleratorKeyPressed(&handler, &mut token);
    });
}

#[derive(Clone, serde::Serialize)]
struct HeldKeys {
    ctrl: bool,
    shift: bool,
    alt: bool,
}

#[link(name = "user32")]
unsafe extern "system" {
    fn GetAsyncKeyState(vkey: i32) -> i16;
}

const VK_SHIFT: i32 = 0x10;
const VK_CONTROL: i32 = 0x11;
const VK_MENU: i32 = 0x12;
const VK_SPACE: i32 = 0x20;
static ALT_SPACE_DOWN: AtomicBool = AtomicBool::new(false);

fn tune_webview(window: &tauri::WebviewWindow) {
    let _ = window.with_webview(|webview| unsafe {
        let Ok(core) = webview.controller().CoreWebView2() else {
            return;
        };
        let Ok(settings) = core.Settings() else {
            return;
        };
        if !cfg!(debug_assertions) {
            let _ = settings.SetAreDevToolsEnabled(false);
        }
        let _ = settings.SetIsStatusBarEnabled(false);
        let _ = settings.SetAreDefaultContextMenusEnabled(false);
        let _ = settings.SetIsZoomControlEnabled(false);
    });
}

const LAUNCHER_WIDTH: f64 = 960.0;
const LAUNCHER_IDLE_HEIGHT: f64 = 132.0;
const LAUNCHER_OPEN_HEIGHT: f64 = 560.0;
// 搜索条停在工作区上方，比贴顶再低一截。
const LAUNCHER_TOP_RATIO: f64 = 0.26;

fn set_launcher_size(window: &tauri::WebviewWindow, height: f64) {
    let scale = window.scale_factor().unwrap_or(1.0);
    let width_px = (LAUNCHER_WIDTH * scale).round() as u32;
    let height_px = (height * scale).round() as u32;
    if let Ok(size) = window.inner_size() {
        if size.width == width_px && size.height == height_px {
            return;
        }
    }
    let _ = window.set_size(tauri::LogicalSize::new(LAUNCHER_WIDTH, height));
}

fn hide_launcher(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if !window.is_visible().unwrap_or(false) {
        return;
    }
    if let Some(state) = app.try_state::<AppState>() {
        state.settings_open.store(false, Ordering::Release);
    }
    let _ = window.hide();
    let _ = window.emit("hidden", ());
    set_launcher_size(&window, LAUNCHER_IDLE_HEIGHT);
}

fn close_before(
    window: &tauri::WebviewWindow,
    action: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    // 先关掉搜索条，再去做可能要等一会儿的启动。失败时再把原来的结果拿回来。
    let app = window.app_handle().clone();
    let _ = window.hide();
    match action() {
        Ok(()) => {
            let _ = window.emit("hidden", ());
            set_launcher_size(window, LAUNCHER_IDLE_HEIGHT);
            Ok(())
        }
        Err(error) => {
            mark_shown(&app);
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
            Err(error)
        }
    }
}

fn place_launcher(window: &tauri::WebviewWindow, height: f64) {
    let Ok(Some(monitor)) = window.current_monitor() else {
        let _ = window.center();
        return;
    };
    let area = monitor.work_area();
    let scale = window.scale_factor().unwrap_or(1.0);
    let width = ((LAUNCHER_WIDTH * scale).round() as u32).min(area.size.width);
    let height_px = ((height * scale).round() as u32).min(area.size.height);
    if let Ok(size) = window.inner_size() {
        if size.width != width || size.height != height_px {
            let _ = window.set_size(tauri::PhysicalSize::new(width, height_px));
        }
    }
    let max_x = area.position.x + area.size.width as i32 - width as i32;
    let max_y = area.position.y + area.size.height as i32 - height_px as i32;
    let x = (area.position.x + (area.size.width as i32 - width as i32) / 2)
        .clamp(area.position.x, max_x.max(area.position.x));
    let y = (area.position.y + (area.size.height as f64 * LAUNCHER_TOP_RATIO) as i32)
        .clamp(area.position.y, max_y.max(area.position.y));
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

fn launcher_open_height(window: &tauri::WebviewWindow) -> f64 {
    let cap = window
        .current_monitor()
        .ok()
        .flatten()
        .map(|monitor| monitor.work_area().size.height as f64 / window.scale_factor().unwrap_or(1.0))
        .unwrap_or(LAUNCHER_OPEN_HEIGHT);
    LAUNCHER_OPEN_HEIGHT.min(cap)
}

fn show_launcher(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    mark_shown(app);
    // 文件列表过期时在后台重扫；唤出本身不等它。
    if let Some(state) = app.try_state::<AppState>() {
        if state.files.enabled.load(Ordering::Acquire)
            && (!state.files.ready.load(Ordering::Acquire) || files_stale(&state))
        {
            spawn_file_scan(app);
        }
    }
    // 新装程序不靠定时重扫：唤出时比一下快捷方式指纹，变了才后台重扫。
    spawn_catalog_check(app);
    set_memory_level(&window, false);
    // 唤出时只显示搜索条。列表区域留到真正展开时再加，避免空窗口被截得很高。
    set_launcher_size(&window, LAUNCHER_IDLE_HEIGHT);
    place_launcher(&window, LAUNCHER_IDLE_HEIGHT);
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    let _ = window.emit("shown", ());
}

fn keep_launcher_on_screen(window: &tauri::WebviewWindow, height: f64) {
    let Ok(Some(monitor)) = window.current_monitor() else {
        return;
    };
    let Ok(position) = window.outer_position() else {
        return;
    };
    let area = monitor.work_area();
    let scale = window.scale_factor().unwrap_or(1.0);
    let height_px = ((height * scale).round() as u32).min(area.size.height);
    let bottom = area.position.y + area.size.height as i32;
    let y = position.y.min(bottom - height_px as i32).max(area.position.y);
    if y != position.y {
        let _ = window.set_position(PhysicalPosition::new(position.x, y));
    }
}

fn toggle_launcher(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        hide_launcher(app);
    } else {
        show_launcher(app);
    }
}

fn remember_shortcut(app: &AppHandle, shortcut: &str) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    if let Ok(mut current) = state.shortcut.lock() {
        *current = shortcut.to_string();
    }
    apply_tray(app);
}

fn apply_tray(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let english = settings::load(app).language == "en";
    let shortcut = state
        .shortcut
        .lock()
        .ok()
        .map(|shortcut| shortcut.clone())
        .unwrap_or_else(|| settings::DEFAULT_SHORTCUT.to_string());
    let label = settings::shortcut_label(&shortcut, english);
    if let Ok(tray) = state.tray.lock() {
        if let Some(tray) = tray.as_ref() {
            let _ = tray.show.set_text(if english { "Show" } else { "显示" });
            let _ = tray
                .settings
                .set_text(if english { "Settings" } else { "设置" });
            let _ = tray.shortcut.set_text(&label);
            let _ = tray.quit.set_text(if english { "Quit" } else { "退出" });
        }
    }
    if let Some(tray) = app.tray_by_id("tray") {
        let _ = tray.set_tooltip(Some(format!("Veery · {label}")));
    }
}

fn saved_shortcut(app: &AppHandle) -> String {
    app.try_state::<AppState>()
        .and_then(|state| state.shortcut.lock().ok().map(|shortcut| shortcut.clone()))
        .unwrap_or_else(|| settings::DEFAULT_SHORTCUT.to_string())
}

fn register_shortcut(app: &AppHandle, shortcut: &str) -> Result<(), String> {
    app.global_shortcut()
        .register(shortcut)
        .map_err(|error| error.to_string())
}

fn ensure_shortcut(app: &AppHandle) {
    let shortcut = saved_shortcut(app);
    let _ = register_shortcut(app, &shortcut);
}

fn show_settings(app: &AppHandle) {
    show_launcher(app);
    let _ = app.emit("open-settings", ());
}

fn install_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示", true, None::<&str>)?;
    let settings_item = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    let shortcut = MenuItem::with_id(app, "shortcut", "", false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &settings_item, &shortcut, &quit])?;
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut slot) = state.tray.lock() {
            *slot = Some(TrayItems {
                show,
                settings: settings_item,
                shortcut,
                quit,
            });
        }
    }
    let tray = TrayIconBuilder::with_id("tray")
        .icon(tauri::include_image!("icons/32x32.png"))
        .tooltip("Veery")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => show_launcher(app),
            "settings" => show_settings(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_launcher(tray.app_handle());
            }
        });
    tray.build(app)?;
    apply_tray(app.handle());
    Ok(())
}

#[derive(serde::Serialize)]
struct SearchResponse {
    ready: bool,
    hits: Vec<catalog::SearchHit>,
}

#[derive(serde::Serialize)]
struct FileHit {
    id: u32,
    name: String,
    icon: Option<String>,
}

#[derive(serde::Serialize)]
struct FileSearchResponse {
    ready: bool,
    hits: Vec<FileHit>,
}

#[tauri::command]
fn search(app: AppHandle, state: tauri::State<AppState>, query: String) -> SearchResponse {
    if let Ok(mut last) = state.last_query.lock() {
        *last = query.clone();
    }
    if !state.index_ready.load(Ordering::Acquire) {
        return SearchResponse {
            ready: false,
            hits: Vec::new(),
        };
    }
    // 历史只读一次；不要放进评分循环里逐条读盘。
    let boosts = history::boosts(&app, &query);
    let mut hits = {
        let apps = state.index.lock().unwrap_or_else(|error| error.into_inner());
        let boost = |_query: &str, path: &Path| {
            boosts
                .get(path.to_string_lossy().as_ref())
                .copied()
                .unwrap_or((0, 0))
        };
        catalog::search_with_boost(&apps, &query, 8, boost)
    };
    attach_icons(&state, &mut hits);
    SearchResponse { ready: true, hits }
}

fn attach_icons(state: &AppState, hits: &mut [catalog::SearchHit]) {
    // 不用 pending 跳过提取：重叠的 search 会把路径标成“提取中”后直接返回空图标，
    // 后完成的那次反而被前端丢掉，同一次查询就不会再要图标了。
    let mut missing = Vec::new();
    {
        let icons = state.icons.lock().unwrap_or_else(|error| error.into_inner());
        for hit in hits.iter_mut() {
            if let Some(icon) = icons.ready.get(&hit.path) {
                hit.icon = Some(icon.clone());
            } else {
                missing.push(hit.path.clone());
            }
        }
    }
    let mut seen = HashSet::new();
    missing.retain(|path| seen.insert(path.clone()));
    let extracted: Vec<_> = missing
        .into_iter()
        .map(|path| {
            let icon = catalog::icon_data_url(&path);
            (path, icon)
        })
        .collect();
    let mut icons = state.icons.lock().unwrap_or_else(|error| error.into_inner());
    for (path, icon) in extracted {
        if let Some(icon) = icon {
            icons.ready.insert(path, icon);
        }
    }
    if icons.ready.len() > 512 {
        let keep: HashSet<_> = hits.iter().map(|hit| hit.path.clone()).collect();
        icons.ready.retain(|path, _| keep.contains(path));
    }
    for hit in hits.iter_mut() {
        if hit.icon.is_none() {
            hit.icon = icons.ready.get(&hit.path).cloned();
        }
    }
}

#[tauri::command]
fn search_files(
    app: AppHandle,
    state: tauri::State<AppState>,
    query: String,
) -> FileSearchResponse {
    if !state.files.enabled.load(Ordering::Acquire) {
        return FileSearchResponse {
            ready: true,
            hits: Vec::new(),
        };
    }
    let types = {
        let types = state
            .files
            .types
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        types.clone()
    };
    // 一个类型都没开时不扫描也不检索，直接当作没有文件结果。
    if types.is_empty() {
        return FileSearchResponse {
            ready: true,
            hits: Vec::new(),
        };
    }
    if !state.files.ready.load(Ordering::Acquire) {
        spawn_file_scan(&app);
        return FileSearchResponse {
            ready: false,
            hits: Vec::new(),
        };
    }
    let picked: Vec<(String, PathBuf)> = {
        let entries = state
            .files
            .entries
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        files::search(&entries, &query, &types, 6)
            .into_iter()
            .map(|entry| (entry.name.clone(), entry.path.clone()))
            .collect()
    };
    let icons = file_icons(&state, &picked);
    let mut hits = Vec::with_capacity(picked.len());
    let mut ids = Vec::with_capacity(picked.len());
    {
        let mut map = state
            .files
            .hits
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if map.len() > FILE_HITS_CAP {
            map.clear();
        }
        for (_, path) in &picked {
            let id = state.files.next_id.fetch_add(1, Ordering::Relaxed);
            map.insert(id, path.clone());
            ids.push(id);
        }
    }
    for ((name, path), id) in picked.into_iter().zip(ids) {
        let icon = icons.get(&path).cloned().flatten();
        hits.push(FileHit { id, name, icon });
    }
    FileSearchResponse { ready: true, hits }
}

fn file_icons(state: &AppState, picked: &[(String, PathBuf)]) -> HashMap<PathBuf, Option<String>> {
    let mut resolved = HashMap::new();
    let mut missing = Vec::new();
    {
        let icons = state
            .files
            .icons
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        for (_, path) in picked {
            match icons.get(path) {
                Some(icon) => {
                    resolved.insert(path.clone(), icon.clone());
                }
                None => missing.push(path.clone()),
            }
        }
    }
    if missing.is_empty() {
        return resolved;
    }
    // 提取放在缓存锁外，避免一次慢图标卡住整个搜索。
    let extracted: Vec<(PathBuf, Option<String>)> = missing
        .into_iter()
        .map(|path| {
            let icon = catalog::icon_data_url(&path);
            (path, icon)
        })
        .collect();
    let mut icons = state
        .files
        .icons
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    if icons.len() + extracted.len() > FILE_ICONS_CAP {
        icons.clear();
    }
    for (path, icon) in extracted {
        icons.insert(path.clone(), icon.clone());
        resolved.insert(path, icon);
    }
    resolved
}

fn file_path(state: &AppState, id: u32) -> Result<PathBuf, String> {
    state
        .files
        .hits
        .lock()
        .map_err(|error| error.to_string())?
        .get(&id)
        .cloned()
        .ok_or_else(|| "找不到这个文件".to_string())
}

#[tauri::command]
fn open_file(
    window: tauri::WebviewWindow,
    state: tauri::State<AppState>,
    id: u32,
) -> Result<(), String> {
    close_before(&window, || {
        let path = file_path(&state, id)?;
        catalog::open_link(&path)
    })
}

#[tauri::command]
fn reveal_file(
    window: tauri::WebviewWindow,
    state: tauri::State<AppState>,
    id: u32,
) -> Result<(), String> {
    close_before(&window, || {
        let path = file_path(&state, id)?;
        catalog::reveal(&path)
    })
}

#[tauri::command]
fn set_file_search(
    app: AppHandle,
    state: tauri::State<AppState>,
    enabled: bool,
) -> Result<(), String> {
    settings::save_file_search(&app, enabled)?;
    state.files.enabled.store(enabled, Ordering::Release);
    if enabled && (!state.files.ready.load(Ordering::Acquire) || files_stale(&state)) {
        spawn_file_scan(&app);
    }
    Ok(())
}

#[tauri::command]
fn set_file_types(
    app: AppHandle,
    state: tauri::State<AppState>,
    types: Vec<String>,
) -> Result<Vec<String>, String> {
    let saved = settings::save_file_types(&app, &types)?;
    if let Ok(mut current) = state.files.types.lock() {
        *current = saved.clone();
    }
    // 之前一个类型都没开时不会扫描；这里补上，不必等下次唤出。
    if !saved.is_empty()
        && state.files.enabled.load(Ordering::Acquire)
        && (!state.files.ready.load(Ordering::Acquire) || files_stale(&state))
    {
        spawn_file_scan(&app);
    }
    Ok(saved)
}

fn store_catalog(state: &AppState, scanned: Result<Vec<catalog::AppEntry>, String>) {
    match scanned {
        Ok(apps) => {
            let paths: HashSet<_> = apps.iter().map(|app| app.path.clone()).collect();
            {
                let mut index = state.index.lock().unwrap_or_else(|error| error.into_inner());
                *index = apps;
            }
            state.index_ready.store(true, Ordering::Release);
            let mut icons = state.icons.lock().unwrap_or_else(|error| error.into_inner());
            icons.ready.retain(|path, _| paths.contains(path));
        }
        Err(error) => {
            // 正式版绝不能往控制台打日志：旧开机项若经 cmd 拉起，用户会看到黑框刷索引文字。
            #[cfg(debug_assertions)]
            eprintln!("veery: 索引扫描失败，继续使用上一份索引: {error}");
            let _ = error;
            if !state.index_ready.load(Ordering::Acquire) {
                state.index_ready.store(true, Ordering::Release);
            }
        }
    }
}

fn reload_catalog(app: &AppHandle) {
    let scanned = catalog::scan();
    #[cfg(debug_assertions)]
    if let Ok(apps) = scanned.as_ref() {
        eprintln!("veery: 已索引 {} 个开始菜单程序", apps.len());
    }
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    store_catalog(&state, scanned);
    let _ = app.emit("catalog-ready", ());
    warm_icons(app);
}

/// 唤出时比对指纹：只列快捷方式，几毫秒；变了再触发全量重扫。
fn spawn_catalog_check(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    // 连按快捷键时只留一个比对线程。
    if state.index_checking.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("veery-index-check".into())
        .spawn(move || {
            let current = catalog::fingerprint();
            let changed = app
                .try_state::<AppState>()
                .map(|state| {
                    let last = *state
                        .index_fingerprint
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    // 指纹还没建立（启动扫描还没完）时不重扫，启动扫描本身会覆盖。
                    matches!((current, last), (Some(current), Some(last)) if current != last)
                })
                .unwrap_or(false);
            if let Some(state) = app.try_state::<AppState>() {
                state.index_checking.store(false, Ordering::Release);
            }
            if changed {
                spawn_catalog_scan(&app);
            }
        });
    if spawned.is_err() {
        state.index_checking.store(false, Ordering::Release);
    }
}

fn spawn_catalog_scan(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    if state.index_scanning.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("veery-index".into())
        .spawn(move || {
            // 先记指纹再扫：扫描期间新增的快捷方式会让下次唤出再补一轮。
            let fingerprint = catalog::fingerprint();
            reload_catalog(&app);
            if let Some(state) = app.try_state::<AppState>() {
                if let Ok(mut last) = state.index_fingerprint.lock() {
                    *last = fingerprint;
                }
                state.index_scanning.store(false, Ordering::Release);
            }
        });
    if spawned.is_err() {
        state.index_scanning.store(false, Ordering::Release);
    }
}

fn warm_icons(app: &AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("veery-icons".into())
        .spawn(move || {
            let Some(state) = app.try_state::<AppState>() else {
                return;
            };
            let paths: Vec<PathBuf> = {
                let apps = state.index.lock().unwrap_or_else(|error| error.into_inner());
                apps.iter().map(|app| app.path.clone()).collect()
            };
            for path in paths {
                {
                    let icons = state.icons.lock().unwrap_or_else(|error| error.into_inner());
                    if icons.ready.contains_key(&path) {
                        continue;
                    }
                }
                let icon = catalog::icon_data_url(&path);
                let mut icons = state.icons.lock().unwrap_or_else(|error| error.into_inner());
                if let Some(icon) = icon {
                    icons.ready.insert(path, icon);
                }
                if icons.ready.len() > 512 {
                    break;
                }
            }
        })
        .ok();
}

#[tauri::command]
fn refresh_index(app: AppHandle) {
    spawn_catalog_scan(&app);
}

fn spawn_file_scan(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    if !state.files.enabled.load(Ordering::Acquire) {
        return;
    }
    {
        let types = state
            .files
            .types
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if types.is_empty() {
            return;
        }
    }
    // 同一时间只跑一轮；开关连点时不会叠出多条扫描线程。
    if state.files.scanning.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("veery-files".into())
        .spawn(move || {
            let scanned = files::scan();
            let Some(state) = app.try_state::<AppState>() else {
                return;
            };
            if let Ok(entries) = scanned {
                let mut cache = state
                    .files
                    .entries
                    .lock()
                    .unwrap_or_else(|error| error.into_inner());
                *cache = entries;
                if let Ok(mut scanned_at) = state.files.scanned_at.lock() {
                    *scanned_at = Some(Instant::now());
                }
                state.files.ready.store(true, Ordering::Release);
            }
            state.files.scanning.store(false, Ordering::Release);
            if state.files.ready.load(Ordering::Acquire) {
                let _ = app.emit("files-ready", ());
            }
        });
    if spawned.is_err() {
        state.files.scanning.store(false, Ordering::Release);
    }
}

fn files_stale(state: &AppState) -> bool {
    state
        .files
        .scanned_at
        .lock()
        .ok()
        .and_then(|scanned_at| *scanned_at)
        .is_none_or(|scanned_at| scanned_at.elapsed() >= FILE_SCAN_STALE)
}

fn path_for(state: &AppState, id: u32) -> Result<PathBuf, String> {
    state
        .index
        .lock()
        .map_err(|error| error.to_string())?
        .iter()
        .find(|app| app.id == id)
        .map(|app| app.path.clone())
        .ok_or_else(|| "找不到这个程序".to_string())
}

#[tauri::command]
fn launch(
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<AppState>,
    id: u32,
    query: Option<String>,
) -> Result<(), String> {
    close_before(&window, || {
        let path = path_for(&state, id)?;
        let remembered = query
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| {
                state
                    .last_query
                    .lock()
                    .map(|value| value.clone())
                    .unwrap_or_default()
            });
        let _ = history::record(&app, &remembered, &path);
        catalog::open_link(&path)
    })
}

#[tauri::command]
fn launch_admin(
    app: AppHandle,
    window: tauri::WebviewWindow,
    state: tauri::State<AppState>,
    id: u32,
    query: Option<String>,
) -> Result<(), String> {
    close_before(&window, || {
        let path = path_for(&state, id)?;
        let remembered = query
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| {
                state
                    .last_query
                    .lock()
                    .map(|value| value.clone())
                    .unwrap_or_default()
            });
        let _ = history::record(&app, &remembered, &path);
        catalog::open_elevated(&path)
    })
}

#[tauri::command]
fn reveal(window: tauri::WebviewWindow, state: tauri::State<AppState>, id: u32) -> Result<(), String> {
    close_before(&window, || {
        let path = path_for(&state, id)?;
        catalog::reveal(&path)
    })
}

#[tauri::command]
fn hide_window(window: tauri::WebviewWindow) {
    hide_launcher(window.app_handle());
}

#[tauri::command]
fn open_settings(app: AppHandle) {
    show_settings(&app);
}

#[tauri::command]
fn set_launcher_frame(window: tauri::WebviewWindow, mode: String) {
    if window.label() != "main" {
        return;
    }
    let settings = mode == "settings";
    if let Some(state) = window.app_handle().try_state::<AppState>() {
        state.settings_open.store(settings, Ordering::Release);
    }
    let height = match mode.as_str() {
        "open" | "settings" => launcher_open_height(&window),
        _ => LAUNCHER_IDLE_HEIGHT,
    };
    set_launcher_size(&window, height);
    if mode != "idle" {
        keep_launcher_on_screen(&window, height);
    }
}

#[tauri::command]
fn get_settings(app: AppHandle) -> settings::SettingsInfo {
    let mut prefs = settings::load(&app);
    prefs.shortcut = saved_shortcut(&app);
    let autostart = settings::autostart_enabled();
    // 两个开关互相独立：界面上的「优先加速」只看用户有没有打开它，不拿系统延迟值反推。
    let fast = autostart && prefs.fast_autostart;
    settings::info(autostart, fast, &prefs)
}

#[tauri::command]
fn set_language(app: AppHandle, language: String) -> Result<(), String> {
    let theme = settings::load(&app).theme;
    let prefs = settings::save_appearance(&app, &language, &theme)?;
    apply_tray(&app);
    let _ = app.emit(
        "appearance",
        serde_json::json!({ "language": prefs.language, "theme": prefs.theme }),
    );
    Ok(())
}

#[tauri::command]
fn set_search_hint(app: AppHandle, hint: String) -> Result<String, String> {
    settings::save_hint(&app, &hint)
}

#[tauri::command]
fn set_theme(app: AppHandle, theme: String) -> Result<(), String> {
    let language = settings::load(&app).language;
    let prefs = settings::save_appearance(&app, &language, &theme)?;
    let _ = app.emit(
        "appearance",
        serde_json::json!({ "language": prefs.language, "theme": prefs.theme }),
    );
    Ok(())
}

#[tauri::command]
async fn check_update() -> Result<update::UpdateInfo, String> {
    let current = env!("CARGO_PKG_VERSION").to_string();
    tauri::async_runtime::spawn_blocking(move || update::check(&current))
        .await
        .map_err(|_| "offline".to_string())?
}

#[tauri::command]
fn open_link(app: AppHandle, url: String) -> Result<(), String> {
    // 关于页打开外链会失焦；短时间内不要因此把窗口藏起来。
    mark_blur_guard(&app);
    settings::open_link(&url)
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    // 用户一动开机项就顺手清旧任务，避免下一次登录还弹黑框。
    settings::spawn_legacy_task_cleanup();
    if !enabled {
        // 关掉开机启动时，优先加速也一并关掉（没有自启就谈不上加速）。
        if settings::load(&app).fast_autostart {
            let _ = settings::apply_fast_autostart(false);
            settings::save_fast_autostart(&app, false)?;
        }
        return settings::set_autostart(false);
    }
    // 只开自启，绝不开优先加速；清掉旧版可能残留的开关状态，避免下次启动又写上延迟。
    if settings::load(&app).fast_autostart {
        settings::save_fast_autostart(&app, false)?;
    }
    settings::set_autostart(true)
}

#[tauri::command]
fn set_fast_autostart(app: AppHandle, enabled: bool) -> Result<(), String> {
    if enabled && !settings::autostart_enabled() {
        return Err("请先打开开机启动".to_string());
    }
    settings::apply_fast_autostart(enabled)?;
    settings::save_fast_autostart(&app, enabled)
}

#[tauri::command]
fn pause_shortcut(app: AppHandle) {
    let _ = app.global_shortcut().unregister_all();
}

#[tauri::command]
fn restore_shortcut(app: AppHandle) {
    ensure_shortcut(&app);
}

#[tauri::command]
fn set_shortcut(app: AppHandle, shortcut: String) -> Result<String, String> {
    let shortcut = shortcut.trim().to_string();
    if !settings::has_modifier(&shortcut) {
        ensure_shortcut(&app);
        return Err("请带上 Ctrl、Alt 或 Shift".to_string());
    }
    let previous = saved_shortcut(&app);
    let _ = app.global_shortcut().unregister_all();
    if let Err(error) = register_shortcut(&app, &shortcut) {
        let _ = register_shortcut(&app, &previous);
        return Err(format!("没有注册成功: {error}"));
    }
    if let Err(error) = settings::save_shortcut(&app, &shortcut) {
        let _ = app.global_shortcut().unregister_all();
        let _ = register_shortcut(&app, &previous);
        return Err(error);
    }
    remember_shortcut(&app, &shortcut);
    let english = settings::load(&app).language == "en";
    Ok(settings::shortcut_label(&shortcut, english))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            // 主窗口隐藏时进程仍在，再次启动只唤醒这一份，不重复注册快捷键。
            show_launcher(app);
        }))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == ShortcutState::Pressed {
                        toggle_launcher(app);
                    }
                })
                .build(),
        )
        .setup(|app| {
            let prefs = settings::load(app.handle());
            let shortcut = prefs.shortcut.clone();
            app.manage(AppState {
                index: Mutex::new(Vec::new()),
                index_ready: AtomicBool::new(false),
                index_scanning: AtomicBool::new(false),
                index_checking: AtomicBool::new(false),
                index_fingerprint: Mutex::new(None),
                icons: Mutex::new(IconCache {
                    ready: HashMap::new(),
                }),
                files: FileState {
                    enabled: AtomicBool::new(prefs.file_search),
                    types: Mutex::new(prefs.file_types.clone()),
                    entries: Mutex::new(Vec::new()),
                    ready: AtomicBool::new(false),
                    scanning: AtomicBool::new(false),
                    scanned_at: Mutex::new(None),
                    hits: Mutex::new(HashMap::new()),
                    icons: Mutex::new(HashMap::new()),
                    next_id: AtomicU32::new(1),
                },
                shown_at: Mutex::new(Instant::now() - Duration::from_secs(60)),
                blur_until: Mutex::new(Instant::now() - Duration::from_secs(60)),
                settings_open: AtomicBool::new(false),
                last_query: Mutex::new(String::new()),
                shortcut: Mutex::new(shortcut.clone()),
                tray: Mutex::new(None),
            });
            spawn_catalog_scan(app.handle());
            if prefs.file_search {
                spawn_file_scan(app.handle());
            }
            if register_shortcut(app.handle(), &shortcut).is_err() {
                let fallback = settings::DEFAULT_SHORTCUT.to_string();
                let _ = register_shortcut(app.handle(), &fallback);
                if let Ok(mut current) = app
                    .try_state::<AppState>()
                    .expect("state")
                    .shortcut
                    .lock()
                {
                    *current = fallback;
                }
            }
            // 先清旧版登录任务，避免继续经 cmd 弹黑框；不挡启动。
            settings::spawn_legacy_task_cleanup();
            if let Err(error) = install_tray(app) {
                #[cfg(debug_assertions)]
                eprintln!("veery: 托盘创建失败: {error}");
                let _ = error;
            }
            // 仍开着开机启动时，校正 Run 路径；优先加速只有用户开过才重写延迟。
            if settings::autostart_enabled() {
                if let Err(error) = settings::set_autostart(true) {
                    #[cfg(debug_assertions)]
                    eprintln!("veery: 开机启动修复失败: {error}");
                    let _ = error;
                }
                if settings::load(app.handle()).fast_autostart {
                    if let Err(error) = settings::apply_fast_autostart(true) {
                        #[cfg(debug_assertions)]
                        eprintln!("veery: 优先加速启动修复失败: {error}");
                        let _ = error;
                    }
                }
            }
            if let Some(window) = app.get_webview_window("main") {
                tune_webview(&window);
                guard_alt_space(&window);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            match event {
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                hide_launcher(window.app_handle());
            }
            WindowEvent::Focused(false) => {
                let app = window.app_handle();
                let state = app.try_state::<AppState>();
                let recent = state
                    .as_ref()
                    .and_then(|state| {
                        state
                            .shown_at
                            .lock()
                            .ok()
                            .map(|shown_at| shown_at.elapsed() < Duration::from_millis(300))
                    })
                    .unwrap_or(false);
                let guarded = state
                    .as_ref()
                    .and_then(|state| state.blur_until.lock().ok())
                    .is_some_and(|until| *until > Instant::now());
                // 点桌面空白应隐藏（含设置中）；打开外链的短时失焦由 blur_until 挡住。
                if !recent && !guarded {
                    hide_launcher(app);
                }
            }
            _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            search,
            search_files,
            open_file,
            reveal_file,
            set_file_search,
            set_file_types,
            refresh_index,
            launch,
            launch_admin,
            reveal,
            hide_window,
            open_settings,
            set_launcher_frame,
            get_settings,
            set_autostart,
            set_fast_autostart,
            set_language,
            set_search_hint,
            set_theme,
            check_update,
            open_link,
            pause_shortcut,
            restore_shortcut,
            set_shortcut
        ])
        .run(tauri::generate_context!())
        .expect("error while running veery");
}
