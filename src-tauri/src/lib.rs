mod catalog;
mod settings;
mod update;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder, WindowEvent, Wry};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{EnumChildWindows, SC_KEYMENU, WM_SYSCOMMAND};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

struct IconCache {
    ready: HashMap<PathBuf, String>,
    pending: HashSet<PathBuf>,
}

struct TrayItems {
    show: MenuItem<Wry>,
    settings: MenuItem<Wry>,
    shortcut: MenuItem<Wry>,
    quit: MenuItem<Wry>,
}

struct AppState {
    index: Mutex<Vec<catalog::AppEntry>>,
    index_ready: AtomicBool,
    icons: Mutex<IconCache>,
    shown_at: Mutex<Instant>,
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
    // 只拦设置窗口自己的 Alt+空格系统菜单，不改其他程序的快捷键。
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

fn settings_title(app: &AppHandle) -> &'static str {
    if settings::load(app).language == "en" {
        "Settings"
    } else {
        "设置"
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
    hide_launcher(app);
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let data_dir = app
        .path()
        .app_local_data_dir()
        .map(|dir| dir.join("settings"))
        .unwrap_or_else(|_| PathBuf::from("settings"));
    if let Err(error) = WebviewWindowBuilder::new(app, "settings", WebviewUrl::App("index.html".into()))
        .title(settings_title(app))
        .inner_size(760.0, 520.0)
        .center()
        .focused(true)
        .resizable(true)
        .visible(true)
        .data_directory(data_dir)
        .icon(tauri::include_image!("icons/128x128.png"))
        .and_then(|builder| builder.build())
    {
        eprintln!("veery: 设置窗口创建失败: {error}");
        return;
    }
    if let Some(window) = app.get_webview_window("settings") {
        guard_alt_space(&window);
    }
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

#[tauri::command]
fn search(state: tauri::State<AppState>, query: String) -> SearchResponse {
    if !state.index_ready.load(Ordering::Acquire) {
        return SearchResponse {
            ready: false,
            hits: Vec::new(),
        };
    }
    let mut hits = {
        let apps = state.index.lock().unwrap_or_else(|error| error.into_inner());
        catalog::search(&apps, &query, 8)
    };
    attach_icons(&state, &mut hits);
    SearchResponse { ready: true, hits }
}

fn attach_icons(state: &AppState, hits: &mut [catalog::SearchHit]) {
    let mut missing = Vec::new();
    {
        let mut icons = state.icons.lock().unwrap_or_else(|error| error.into_inner());
        for hit in hits.iter_mut() {
            if let Some(icon) = icons.ready.get(&hit.path) {
                hit.icon = Some(icon.clone());
            } else if icons.pending.insert(hit.path.clone()) {
                missing.push(hit.path.clone());
            }
        }
    }
    let extracted: Vec<_> = missing
        .into_iter()
        .map(|path| {
            let icon = catalog::icon_data_url(&path);
            (path, icon)
        })
        .collect();
    let mut icons = state.icons.lock().unwrap_or_else(|error| error.into_inner());
    for (path, icon) in extracted {
        icons.pending.remove(&path);
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
            icons.pending.retain(|path| paths.contains(path));
        }
        Err(error) => {
            eprintln!("veery: 索引扫描失败，继续使用上一份索引: {error}");
            if !state.index_ready.load(Ordering::Acquire) {
                state.index_ready.store(true, Ordering::Release);
            }
        }
    }
}

fn reload_catalog(app: &AppHandle) {
    let scanned = catalog::scan();
    if let Ok(apps) = scanned.as_ref() {
        eprintln!("veery: 已索引 {} 个开始菜单程序", apps.len());
    }
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    store_catalog(&state, scanned);
    let _ = app.emit("catalog-ready", ());
}

#[tauri::command]
fn refresh_index(app: AppHandle) {
    let app = app.clone();
    std::thread::Builder::new()
        .name("veery-index".into())
        .spawn(move || reload_catalog(&app))
        .ok();
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
fn launch(window: tauri::WebviewWindow, state: tauri::State<AppState>, id: u32) -> Result<(), String> {
    close_before(&window, || {
        let path = path_for(&state, id)?;
        catalog::open_link(&path)
    })
}

#[tauri::command]
fn launch_admin(
    window: tauri::WebviewWindow,
    state: tauri::State<AppState>,
    id: u32,
) -> Result<(), String> {
    close_before(&window, || {
        let path = path_for(&state, id)?;
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
fn set_launcher_open(window: tauri::WebviewWindow, open: bool) {
    if window.label() != "main" {
        return;
    }
    let height = if open {
        launcher_open_height(&window)
    } else {
        LAUNCHER_IDLE_HEIGHT
    };
    set_launcher_size(&window, height);
    if open {
        keep_launcher_on_screen(&window, height);
    }
}

#[tauri::command]
fn get_settings(app: AppHandle) -> settings::SettingsInfo {
    let mut prefs = settings::load(&app);
    prefs.shortcut = saved_shortcut(&app);
    settings::info(settings::autostart_enabled(), &prefs)
}

#[tauri::command]
fn set_language(app: AppHandle, language: String) -> Result<(), String> {
    let theme = settings::load(&app).theme;
    let prefs = settings::save_appearance(&app, &language, &theme)?;
    apply_tray(&app);
    if let Some(window) = app.get_webview_window("settings") {
        let _ = window.set_title(settings_title(&app));
    }
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
fn open_link(url: String) -> Result<(), String> {
    settings::open_link(&url)
}

#[tauri::command]
fn set_autostart(enabled: bool) -> Result<(), String> {
    settings::set_autostart(enabled)
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
            let shortcut = settings::load_shortcut(app.handle());
            app.manage(AppState {
                index: Mutex::new(Vec::new()),
                index_ready: AtomicBool::new(false),
                icons: Mutex::new(IconCache {
                    ready: HashMap::new(),
                    pending: HashSet::new(),
                }),
                shown_at: Mutex::new(Instant::now() - Duration::from_secs(60)),
                shortcut: Mutex::new(shortcut.clone()),
                tray: Mutex::new(None),
            });
            let handle = app.handle().clone();
            std::thread::Builder::new()
                .name("veery-index".into())
                .spawn(move || reload_catalog(&handle))
                .ok();
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
            if let Err(error) = install_tray(app) {
                eprintln!("veery: 托盘创建失败: {error}");
            }
            if let Some(window) = app.get_webview_window("main") {
                tune_webview(&window);
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "settings" {
                if matches!(event, WindowEvent::Destroyed) {
                    ensure_shortcut(window.app_handle());
                }
                return;
            }
            if window.label() != "main" {
                return;
            }
            match event {
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                hide_launcher(window.app_handle());
            }
            WindowEvent::Focused(false) => {
                let recent = window
                    .app_handle()
                    .try_state::<AppState>()
                    .and_then(|state| {
                        state
                            .shown_at
                            .lock()
                            .ok()
                            .map(|shown_at| shown_at.elapsed() < Duration::from_millis(300))
                    })
                    .unwrap_or(false);
                if !recent {
                    hide_launcher(window.app_handle());
                }
            }
            _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            search,
            refresh_index,
            launch,
            launch_admin,
            reveal,
            hide_window,
            set_launcher_open,
            get_settings,
            set_autostart,
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
