mod catalog;
mod settings;

use std::collections::HashMap;
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

struct AppState {
    index: Mutex<Vec<catalog::AppEntry>>,
    icons: Mutex<HashMap<PathBuf, Option<String>>>,
    shown_at: Mutex<Instant>,
    shortcut: Mutex<String>,
    shortcut_item: Mutex<Option<MenuItem<Wry>>>,
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

fn hide_launcher(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    if !window.is_visible().unwrap_or(false) {
        return;
    }
    let _ = window.hide();
    let _ = window.emit("hidden", ());
    set_memory_level(&window, true);
}

fn place_launcher(window: &tauri::WebviewWindow) {
    let Ok(Some(monitor)) = window.current_monitor() else {
        let _ = window.center();
        return;
    };
    let area = monitor.work_area();
    let Ok(size) = window.outer_size() else {
        let _ = window.center();
        return;
    };
    let x = area.position.x + (area.size.width as i32 - size.width as i32) / 2;
    let y = area.position.y + (area.size.height as f64 * 0.18) as i32;
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

fn show_launcher(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    mark_shown(app);
    set_memory_level(&window, false);
    let _ = window.set_size(tauri::LogicalSize::new(960.0, 132.0));
    place_launcher(&window);
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
    let _ = window.emit("shown", ());
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
    let label = settings::shortcut_label(shortcut);
    if let Ok(item) = state.shortcut_item.lock() {
        if let Some(item) = item.as_ref() {
            let _ = item.set_text(&label);
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
        .title("设置")
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

fn install_tray(app: &tauri::App, shortcut_label: &str) -> tauri::Result<()> {
    let show = MenuItem::with_id(app, "show", "显示", true, None::<&str>)?;
    let settings_item = MenuItem::with_id(app, "settings", "设置", true, None::<&str>)?;
    let shortcut = MenuItem::with_id(app, "shortcut", shortcut_label, false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "退出", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &settings_item, &shortcut, &quit])?;
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut slot) = state.shortcut_item.lock() {
            *slot = Some(shortcut);
        }
    }
    let tray = TrayIconBuilder::with_id("tray")
        .icon(tauri::include_image!("icons/32x32.png"))
        .tooltip(format!("Veery · {shortcut_label}"))
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
    Ok(())
}

#[tauri::command]
fn search(state: tauri::State<AppState>, query: String) -> Vec<catalog::SearchHit> {
    let mut hits = {
        let apps = state.index.lock().unwrap_or_else(|error| error.into_inner());
        catalog::search(&apps, &query, 8)
    };
    let mut icons = state.icons.lock().unwrap_or_else(|error| error.into_inner());
    for hit in &mut hits {
        hit.icon = icons
            .entry(hit.path.clone())
            .or_insert_with(|| catalog::icon_data_url(&hit.path))
            .clone();
    }
    hits
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
    let path = path_for(&state, id)?;
    catalog::open_link(&path)?;
    hide_launcher(window.app_handle());
    Ok(())
}

#[tauri::command]
fn launch_admin(
    window: tauri::WebviewWindow,
    state: tauri::State<AppState>,
    id: u32,
) -> Result<(), String> {
    let path = path_for(&state, id)?;
    catalog::open_elevated(&path)?;
    hide_launcher(window.app_handle());
    Ok(())
}

#[tauri::command]
fn reveal(window: tauri::WebviewWindow, state: tauri::State<AppState>, id: u32) -> Result<(), String> {
    let path = path_for(&state, id)?;
    catalog::reveal(&path)?;
    hide_launcher(window.app_handle());
    Ok(())
}

#[tauri::command]
fn hide_window(window: tauri::WebviewWindow) {
    hide_launcher(window.app_handle());
}

#[tauri::command]
fn get_settings(app: AppHandle) -> settings::SettingsInfo {
    let shortcut = saved_shortcut(&app);
    settings::info(settings::autostart_enabled(), &shortcut)
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
    settings::save_shortcut(&app, &shortcut)?;
    remember_shortcut(&app, &shortcut);
    Ok(settings::shortcut_label(&shortcut))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
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
                icons: Mutex::new(HashMap::new()),
                shown_at: Mutex::new(Instant::now() - Duration::from_secs(60)),
                shortcut: Mutex::new(shortcut.clone()),
                shortcut_item: Mutex::new(None),
            });
            let handle = app.handle().clone();
            std::thread::Builder::new()
                .name("veery-index".into())
                .spawn(move || {
                    let apps = catalog::scan();
                    eprintln!("veery: 已索引 {} 个开始菜单程序", apps.len());
                    if let Some(state) = handle.try_state::<AppState>() {
                        let mut index = state.index.lock().unwrap_or_else(|error| error.into_inner());
                        *index = apps;
                    }
                })
                .ok();
            let registered = if register_shortcut(app.handle(), &shortcut).is_ok() {
                shortcut
            } else {
                let fallback = settings::DEFAULT_SHORTCUT.to_string();
                let _ = register_shortcut(app.handle(), &fallback);
                if let Ok(mut current) = app
                    .try_state::<AppState>()
                    .expect("state")
                    .shortcut
                    .lock()
                {
                    *current = fallback.clone();
                }
                fallback
            };
            let shortcut_label = settings::shortcut_label(&registered);
            if let Err(error) = install_tray(app, &shortcut_label) {
                eprintln!("veery: 托盘创建失败: {error}");
            }
            if let Some(window) = app.get_webview_window("main") {
                tune_webview(&window);
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(2));
                    let Some(window) = handle.get_webview_window("main") else {
                        return;
                    };
                    if !window.is_visible().unwrap_or(false) {
                        set_memory_level(&window, true);
                    }
                });
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
            launch,
            launch_admin,
            reveal,
            hide_window,
            get_settings,
            set_autostart,
            pause_shortcut,
            restore_shortcut,
            set_shortcut
        ])
        .run(tauri::generate_context!())
        .expect("error while running veery");
}
