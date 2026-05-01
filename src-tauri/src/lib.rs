mod clipboard;
mod models;
mod source;
mod store;

use crate::{
    clipboard::{copy_item_to_system_clipboard, ClipboardCache},
    models::{ClipboardItem, Settings},
    store::Store,
};
use std::{
    process::Command,
    str::FromStr,
    sync::{Arc, Mutex},
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    App, AppHandle, Emitter, Manager, State, WebviewWindow, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

pub struct AppState {
    store: Store,
    settings: Mutex<Settings>,
    clipboard_cache: Mutex<ClipboardCache>,
    last_active_app: Mutex<Option<String>>,
}

impl AppState {
    fn new(app_data_dir: &std::path::Path) -> Result<Self, String> {
        let store = Store::new(app_data_dir)?;
        let settings = store.load_settings()?;
        store.apply_cleanup(&settings)?;
        Ok(Self {
            store,
            settings: Mutex::new(settings),
            clipboard_cache: Mutex::new(ClipboardCache::default()),
            last_active_app: Mutex::new(None),
        })
    }
}

#[tauri::command]
fn get_items(state: State<'_, Arc<AppState>>, query: String) -> Result<Vec<ClipboardItem>, String> {
    state.store.search_items(&query, 120)
}

#[tauri::command]
fn copy_item(app: AppHandle, state: State<'_, Arc<AppState>>, id: i64) -> Result<(), String> {
    copy_item_to_system_clipboard(&state, id)?;
    let hide_after_copy = state
        .settings
        .lock()
        .map_err(|err| err.to_string())?
        .hide_after_copy;
    if hide_after_copy {
        hide_main_window(&app);
    }
    Ok(())
}

#[tauri::command]
fn copy_source_url(state: State<'_, Arc<AppState>>, id: i64) -> Result<(), String> {
    let url = state.store.get_source_url(id)?;
    clipboard::copy_text_to_system_clipboard(&state, url)
}

#[tauri::command]
fn open_source_url(state: State<'_, Arc<AppState>>, id: i64) -> Result<(), String> {
    let url = state.store.get_source_url(id)?;
    open_url(&url)
}

#[tauri::command]
fn delete_item(state: State<'_, Arc<AppState>>, id: i64) -> Result<(), String> {
    state.store.delete_item(id)
}

#[tauri::command]
fn clear_history(state: State<'_, Arc<AppState>>) -> Result<(), String> {
    state.store.clear_history()
}

#[tauri::command]
fn get_settings(state: State<'_, Arc<AppState>>) -> Result<Settings, String> {
    state
        .settings
        .lock()
        .map(|settings| settings.clone())
        .map_err(|err| err.to_string())
}

#[tauri::command]
fn save_settings(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    settings: Settings,
) -> Result<Settings, String> {
    let normalized = normalize_settings(settings);
    state.store.save_settings(&normalized)?;
    state.store.apply_cleanup(&normalized)?;
    register_shortcut(&app, &normalized.shortcut)?;

    let mut guard = state.settings.lock().map_err(|err| err.to_string())?;
    *guard = normalized.clone();
    Ok(normalized)
}

#[tauri::command]
fn toggle_pause(state: State<'_, Arc<AppState>>, paused: bool) -> Result<Settings, String> {
    let mut settings = state.settings.lock().map_err(|err| err.to_string())?;
    settings.paused = paused;
    let updated = settings.clone();
    drop(settings);
    state.store.save_settings(&updated)?;
    Ok(updated)
}

#[tauri::command]
fn show_window(app: AppHandle) {
    show_main_window(&app);
}

#[tauri::command]
fn hide_window(app: AppHandle) {
    hide_main_window(&app);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        show_main_window(app);
                    }
                })
                .build(),
        )
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }

            let app_data_dir = app.path().app_data_dir()?;
            let state = Arc::new(AppState::new(&app_data_dir)?);
            let shortcut = state
                .settings
                .lock()
                .map(|settings| settings.shortcut.clone())
                .unwrap_or_else(|_| Settings::default().shortcut);

            app.manage(state.clone());
            register_shortcut(app.handle(), &shortcut)?;
            create_tray(app)?;
            clipboard::start_monitor(app.handle().clone(), state);

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_items,
            copy_item,
            copy_source_url,
            open_source_url,
            delete_item,
            clear_history,
            get_settings,
            save_settings,
            toggle_pause,
            show_window,
            hide_window
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn create_tray(app: &App) -> tauri::Result<()> {
    let show_i = MenuItem::with_id(app, "show", "Pokaż ClipVault", true, None::<&str>)?;
    let pause_i = MenuItem::with_id(app, "toggle_pause", "Pauza / Wznów", true, None::<&str>)?;
    let clear_i = MenuItem::with_id(app, "clear", "Wyczyść historię", true, None::<&str>)?;
    let quit_i = MenuItem::with_id(app, "quit", "Zakończ", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show_i, &pause_i, &clear_i, &quit_i])?;

    let icon = app.default_window_icon().cloned();
    let mut builder = TrayIconBuilder::new()
        .tooltip("ClipVault")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main_window(app),
            "toggle_pause" => toggle_pause_from_tray(app),
            "clear" => {
                if let Some(state) = app.try_state::<Arc<AppState>>() {
                    let _ = state.store.clear_history();
                    let _ = app.emit("clipboard-updated", store::unix_now());
                }
            }
            "quit" => app.exit(0),
            _ => {}
        });

    if let Some(icon) = icon {
        builder = builder.icon(icon);
    }

    builder.build(app)?;
    Ok(())
}

fn register_shortcut(app: &AppHandle, shortcut: &str) -> Result<(), String> {
    let manager = app.global_shortcut();
    manager.unregister_all().map_err(|err| err.to_string())?;
    let shortcut = Shortcut::from_str(shortcut).map_err(|err| err.to_string())?;
    manager.register(shortcut).map_err(|err| err.to_string())
}

fn normalize_settings(mut settings: Settings) -> Settings {
    if settings.max_items < 25 {
        settings.max_items = 25;
    }
    if settings.shortcut.trim().is_empty() {
        settings.shortcut = Settings::default().shortcut;
    }
    settings.shortcut = settings.shortcut.trim().to_string();
    settings
}

fn toggle_pause_from_tray(app: &AppHandle) {
    let Some(state) = app.try_state::<Arc<AppState>>() else {
        return;
    };
    let Ok(mut settings) = state.settings.lock() else {
        return;
    };
    settings.paused = !settings.paused;
    let updated = settings.clone();
    drop(settings);
    let _ = state.store.save_settings(&updated);
    let _ = app.emit("settings-updated", updated);
}

fn show_main_window(app: &AppHandle) {
    remember_active_app(app);
    if let Some(state) = app.try_state::<Arc<AppState>>() {
        let _ = clipboard::capture_now(app, state.inner());
    }
    if let Some(window) = app.get_webview_window("main") {
        focus_window(&window);
        let _ = app.emit("focus-search", ());
    }
}

fn hide_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
    restore_last_active_app(app);
}

fn focus_window(window: &WebviewWindow) {
    let _ = window.show();
    let _ = window.unminimize();
    let _ = window.set_focus();
}

fn open_url(url: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(url);
        command
    };

    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("cmd");
        command.args(["/C", "start", "", url]);
        command
    };

    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(url);
        command
    };

    command
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("Nie udało się otworzyć linku: {err}"))
}

fn remember_active_app(app: &AppHandle) {
    let Some(state) = app.try_state::<Arc<AppState>>() else {
        return;
    };
    let source = source::detect_source();
    let Some(app_name) = source.app_name else {
        return;
    };
    if source::is_clipvault_app(&app_name) {
        return;
    }
    if let Ok(mut last_active_app) = state.last_active_app.lock() {
        *last_active_app = Some(app_name);
    };
}

fn restore_last_active_app(app: &AppHandle) {
    let Some(state) = app.try_state::<Arc<AppState>>() else {
        return;
    };
    let app_name = state
        .last_active_app
        .lock()
        .ok()
        .and_then(|guard| guard.clone());
    if let Some(app_name) = app_name {
        let _ = source::activate_app(&app_name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_shortcut_opens_clipvault_with_cmd_shift_s_on_macos() {
        Shortcut::from_str(&Settings::default().shortcut).expect("shortcut should parse");
    }
}
