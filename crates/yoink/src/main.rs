mod app;
mod assets;
mod ipc;
mod text;
mod tray;
mod x11;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, mpsc};

use futures::StreamExt as _;
use gpui_kit::*;
use yoink_core::{Config, HistoryStore};

use crate::app::{Message, Yoink};
use crate::ipc::Request;

const SOCKET: &str = "yoink.sock";
const LOCK: &str = "yoink.lock";

const USAGE: &str = "yoink - clipboard history for Linux X11

Usage: yoink [--hidden | --quit | --version | --help]

Shows the window, and starts yoink first if it is not running.
Run it again while the window has focus to hide the window.
--hidden starts yoink without showing the window, for autostart.
--quit stops the running instance without changing history.";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let hidden = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["--help"] | ["-h"] => return println!("{USAGE}"),
        ["--version"] | ["-V"] => return println!("yoink {}", env!("CARGO_PKG_VERSION")),
        [] => false,
        ["--hidden"] => true,
        ["--quit"] => {
            ipc::send(&ipc::runtime_dir(&data_dir()).join(SOCKET), Request::Quit);
            return;
        }
        _ => {
            eprintln!("yoink: unknown arguments; run yoink --help");
            std::process::exit(2);
        }
    };
    let runtime_dir = ipc::runtime_dir(&data_dir());
    let socket = runtime_dir.join(SOCKET);
    // The fast path: a running instance does the work and this process exits
    // before loading any UI code.
    if ipc::send(
        &socket,
        if hidden {
            Request::Ping
        } else {
            Request::Toggle
        },
    ) {
        return;
    }
    if let Err(error) = run(hidden, &socket, &runtime_dir.join(LOCK)) {
        eprintln!("yoink: {error}");
        std::process::exit(1);
    }
}

fn xdg_dir(variable: &str, fallback: &str) -> PathBuf {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(fallback)
        })
        .join("dev.garyj.yoink")
}

/// Earlier releases keep history in this directory, so keep its name.
fn data_dir() -> PathBuf {
    xdg_dir("XDG_DATA_HOME", ".local/share")
}

fn run(hidden: bool, socket: &Path, lock: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let data_dir = data_dir();
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&data_dir)?;
        std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o700))?;
    }
    let config_path = xdg_dir("XDG_CONFIG_HOME", ".config").join("config.toml");
    let config =
        Config::load(&config_path).map_err(|e| format!("{}: {e}", config_path.display()))?;

    let Some(lock) = ipc::lock(lock)? else {
        // Another launch holds the lock and is still starting; hand over once
        // it listens.
        if !hidden {
            for _ in 0..100 {
                if ipc::send(socket, Request::Toggle) {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
        return Ok(());
    };
    let listener = ipc::bind(socket, &lock)?;

    let mut store = HistoryStore::open(&data_dir.join("history.sqlite"))?;
    store.set_max_items(config.max_items)?;
    let store = Arc::new(Mutex::new(store));

    let (messages, mut inbox) = futures::channel::mpsc::unbounded();
    {
        let messages = messages.clone();
        ipc::serve(listener, move |request| {
            let _ = messages.unbounded_send(Message::Request(request));
        });
    }
    let (clipboard_tx, clipboard_rx) = mpsc::channel();
    {
        let messages = messages.clone();
        yoink_clipboard::spawn(store.clone(), clipboard_rx, move || {
            let _ = messages.unbounded_send(Message::HistoryChanged);
        });
    }
    {
        let messages = messages.clone();
        std::thread::spawn(move || {
            tray::spawn(move |request| {
                let _ = messages.unbounded_send(Message::Request(request));
            })
        });
    }
    let x11 = x11::spawn_worker();

    gpui_kit::application()
        .with_assets(assets::AppAssets)
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx| {
            gpui_kit::init(cx);
            app::bind_keys(cx);
            app::apply_theme(None, cx);
            let window_size = app::window_size();
            let options = WindowOptions {
                // GPUI maps a new window at once. Off screen, that map is never
                // seen. The first show moves the window to the pointer's monitor.
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    point(px(-16000.), px(-16000.)),
                    window_size,
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some("yoink".into()),
                    ..Default::default()
                }),
                app_id: Some("yoink".into()),
                window_background: WindowBackgroundAppearance::Transparent,
                window_decorations: Some(WindowDecorations::Client),
                window_min_size: Some(size(px(560.), px(360.))),
                ..Default::default()
            };
            let opened = gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| Yoink::new(store, clipboard_tx, x11, !hidden, window, cx))
            });
            let (handle, view) = match opened {
                Ok(opened) => opened,
                Err(error) => {
                    eprintln!("yoink: cannot open the window: {error}");
                    cx.quit();
                    return;
                }
            };
            handle
                .update(cx, |_, window, cx| {
                    let closing = view.clone();
                    window.on_window_should_close(cx, move |window, cx| {
                        closing.update(cx, |view, cx| view.hide(window, cx));
                        false
                    });
                    view.update(cx, |view, cx| {
                        let request = if hidden { Request::Hide } else { Request::Show };
                        view.handle(Message::Request(request), window, cx);
                    });
                })
                .ok();
            cx.spawn(async move |cx| {
                while let Some(message) = inbox.next().await {
                    let view = view.clone();
                    let delivered = handle.update(cx, |_, window, cx| {
                        view.update(cx, |view, cx| view.handle(message, window, cx))
                    });
                    if delivered.is_err() {
                        break;
                    }
                }
            })
            .detach();
        });
    Ok(())
}
