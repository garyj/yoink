mod clipboard;

use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use serde::Serialize;
use tauri::{AppHandle, Manager};
use yoink_core::{Config, HistoryStore, ItemKind};

const PAGE_SIZE: usize = 50;
struct AppState {
    store: Arc<Mutex<HistoryStore>>,
    clipboard_tx: mpsc::Sender<clipboard::Write>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ClipItemDto {
    id: i64,
    kind: &'static str,
    text: String,
    thumb: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    last_copied_at_ms: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HistoryPage {
    items: Vec<ClipItemDto>,
    total: usize,
    page_size: usize,
}

#[tauri::command]
fn history(
    query: String,
    offset: usize,
    state: tauri::State<AppState>,
) -> Result<HistoryPage, String> {
    let store = state.store.lock().unwrap();
    let total = store.count_matches(&query).map_err(|e| e.to_string())?;
    let items = store
        .preview_page(&query, PAGE_SIZE, offset)
        .map_err(|e| e.to_string())?;
    let items = items
        .into_iter()
        .map(|item| {
            let thumb = match item.kind {
                ItemKind::Text => None,
                ItemKind::Image => {
                    store
                        .thumbnail(item.id)
                        .map_err(|e| e.to_string())?
                        .map(|bytes| {
                            let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
                            format!("data:image/png;base64,{b64}")
                        })
                }
            };
            Ok(ClipItemDto {
                id: item.id,
                kind: match item.kind {
                    ItemKind::Text => "text",
                    ItemKind::Image => "image",
                },
                text: item.text,
                thumb,
                width: item.width,
                height: item.height,
                last_copied_at_ms: item.last_copied_at_ms,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(HistoryPage {
        items,
        total,
        page_size: PAGE_SIZE,
    })
}

#[tauri::command]
async fn yoink(
    id: i64,
    window: tauri::WebviewWindow,
    state: tauri::State<'_, AppState>,
) -> Result<(), String> {
    let (item, png) = {
        let store = state.store.lock().unwrap();
        let item = store
            .get(id)
            .map_err(|e| e.to_string())?
            .ok_or("item no longer exists")?;
        let png = match item.kind {
            ItemKind::Text => None,
            ItemKind::Image => Some(
                store
                    .image_data(id)
                    .map_err(|e| e.to_string())?
                    .ok_or("image data missing")?,
            ),
        };
        (item, png)
    };

    let content = match png {
        None => clipboard::Content::Text(item.text),
        Some(png) => clipboard::decode_image(png)?,
    };
    let (reply, result) = mpsc::channel();
    state
        .clipboard_tx
        .send(clipboard::Write { content, reply })
        .map_err(|_| "clipboard worker is unavailable")?;
    tauri::async_runtime::spawn_blocking(move || result.recv())
        .await
        .map_err(|e| e.to_string())?
        .map_err(|_| "clipboard worker stopped before completing the copy")??;
    let _ = window.hide();
    Ok(())
}

#[tauri::command]
fn delete_item(id: i64, state: tauri::State<AppState>) -> Result<bool, String> {
    let mut store = state.store.lock().unwrap();
    store.remove(id).map_err(|e| e.to_string())
}

#[tauri::command]
fn clear_history(state: tauri::State<AppState>) -> Result<(), String> {
    let mut store = state.store.lock().unwrap();
    store.clear().map_err(|e| e.to_string())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Hide only when the window is visible and focused. A buried or minimized
/// window is re-mapped instead: a fresh map is placed on top by the window
/// manager, which a plain raise from a background event cannot achieve under
/// focus-stealing prevention.
fn toggle_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let visible = window.is_visible().unwrap_or(false) && !window.is_minimized().unwrap_or(false);
    let focused = window.is_focused().unwrap_or(false);
    if visible && focused {
        let _ = window.hide();
    } else {
        let _ = window.hide();
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        activate_x11(&window);
    }
}

/// Ported from CopyQ's X11PlatformWindow::raise(): activation requested via a
/// _NET_ACTIVE_WINDOW client message with source indication 2 (user action,
/// what taskbars and pagers send). The window manager honors that where it
/// denies a plain focus request from a background event, which otherwise
/// leaves the window mapped behind others and unfocused, making the toggle
/// need a second press.
fn activate_x11(window: &tauri::WebviewWindow) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{
        ClientMessageEvent, ConfigureWindowAux, ConnectionExt, EventMask, InputFocus, StackMode,
    };

    let Ok(handle) = window.window_handle() else {
        return;
    };
    let xid = match handle.as_raw() {
        RawWindowHandle::Xlib(h) => h.window as u32,
        RawWindowHandle::Xcb(h) => h.window.get(),
        _ => return,
    };
    let result: Result<(), Box<dyn std::error::Error>> = (|| {
        let (conn, screen_num) = x11rb::connect(None)?;
        let root = conn.setup().roots[screen_num].root;
        let atom = conn
            .intern_atom(false, b"_NET_ACTIVE_WINDOW")?
            .reply()?
            .atom;
        let event = ClientMessageEvent::new(32, xid, atom, [2, x11rb::CURRENT_TIME, 0, 0, 0]);
        conn.send_event(
            false,
            root,
            EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT,
            event,
        )?;
        conn.configure_window(xid, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE))?;
        conn.set_input_focus(InputFocus::POINTER_ROOT, xid, x11rb::CURRENT_TIME)?;
        conn.flush()?;
        Ok(())
    })();
    if let Err(e) = result {
        eprintln!("yoink: X11 activation failed: {e}");
    }
}

/// StatusNotifierItem tray via ksni rather than Tauri's built-in tray:
/// libappindicator (Tauri's Linux backend) is menu-only, while a native SNI
/// item receives Activate on left click, which is how CopyQ opens its window
/// from the panel.
struct YoinkTray {
    app: AppHandle,
}

impl YoinkTray {
    fn toggle(&self) {
        let handle = self.app.clone();
        let _ = self.app.run_on_main_thread(move || toggle_window(&handle));
    }
}

impl ksni::Tray for YoinkTray {
    fn id(&self) -> String {
        "yoink".into()
    }

    fn title(&self) -> String {
        "yoink".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        static ICON_PNG: &[u8] = include_bytes!("../icons/32x32.png");
        let Ok(decoded) = image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png)
        else {
            return Vec::new();
        };
        let rgba = decoded.to_rgba8();
        let (width, height) = (rgba.width() as i32, rgba.height() as i32);
        let mut data = rgba.into_raw();
        // RGBA to ARGB32 in network byte order, per the SNI spec.
        for pixel in data.as_chunks_mut::<4>().0 {
            pixel.rotate_right(1);
        }
        vec![ksni::Icon {
            width,
            height,
            data,
        }]
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.toggle();
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        use ksni::menu::StandardItem;
        vec![
            StandardItem {
                label: "Show / Hide".into(),
                activate: Box::new(|tray: &mut Self| tray.toggle()),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Quit".into(),
                activate: Box::new(|tray: &mut Self| tray.app.exit(0)),
                ..Default::default()
            }
            .into(),
        ]
    }
}

pub fn run(quit: bool) {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if argv.get(1).is_some_and(|arg| arg == "--quit") {
                app.exit(0);
            } else {
                toggle_window(app);
            }
        }))
        .setup(move |app| {
            if quit {
                app.handle().exit(0);
                return Ok(());
            }
            let config_path = app.path().app_config_dir()?.join("config.toml");
            let config = Config::load(&config_path)
                .map_err(|e| format!("{}: {e}", config_path.display()))?;

            let data_dir = app.path().app_data_dir()?;
            {
                use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
                std::fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(&data_dir)?;
                std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o700))?;
            }
            let mut store = HistoryStore::open(&data_dir.join("history.sqlite"))?;
            store.set_max_items(config.max_items)?;
            let store = Arc::new(Mutex::new(store));

            let (clipboard_tx, clipboard_rx) = mpsc::channel();
            clipboard::spawn(app.handle().clone(), store.clone(), clipboard_rx);
            app.manage(AppState {
                store,
                clipboard_tx,
            });

            {
                use ksni::blocking::TrayMethods;
                let tray = YoinkTray {
                    app: app.handle().clone(),
                };
                if let Err(e) = tray.spawn() {
                    eprintln!("yoink: failed to create tray icon: {e}");
                }
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            history,
            yoink,
            delete_item,
            clear_history
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
