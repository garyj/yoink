use std::borrow::Cow;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use image::codecs::png::PngEncoder;
use image::{ImageEncoder, RgbaImage};
use tauri::{AppHandle, Emitter};
use x11rb::connection::Connection;
use x11rb::protocol::xfixes::{ConnectionExt as _, SelectionEventMask};
use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt as _, CreateWindowAux, WindowClass};
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use yoink_core::HistoryStore;

const POLL_INTERVAL: Duration = Duration::from_millis(100);
const MAX_TEXT_BYTES: usize = 1024 * 1024;
const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: usize = 8192;
const MAX_IMAGE_PNG_BYTES: usize = 10 * 1024 * 1024;

pub(crate) enum Content {
    Text(String),
    Image {
        rgba: Vec<u8>,
        width: u32,
        height: u32,
    },
}

pub(crate) struct Write {
    pub content: Content,
    pub reply: mpsc::Sender<Result<(), String>>,
}

pub(crate) fn decode_image(png: Vec<u8>) -> Result<Content, String> {
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION as u32);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION as u32);
    limits.max_alloc = Some(MAX_IMAGE_BYTES as u64);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|e| e.to_string())?.to_rgba8();
    if !valid_image_size(
        decoded.width() as usize,
        decoded.height() as usize,
        decoded.as_raw().len(),
    ) {
        return Err("image exceeds the 64 MiB decoded size limit".into());
    }
    Ok(Content::Image {
        width: decoded.width(),
        height: decoded.height(),
        rgba: decoded.into_raw(),
    })
}

fn valid_image_size(width: usize, height: usize, bytes: usize) -> bool {
    width > 0
        && height > 0
        && width <= MAX_IMAGE_DIMENSION
        && height <= MAX_IMAGE_DIMENSION
        && bytes <= MAX_IMAGE_BYTES
        && width.checked_mul(height).and_then(|n| n.checked_mul(4)) == Some(bytes)
}

fn encode_png(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let mut png = Vec::new();
    PngEncoder::new(&mut png)
        .write_image(rgba, width, height, image::ExtendedColorType::Rgba8)
        .map_err(|e| e.to_string())?;
    Ok(png)
}

// Keep the only arboard owner alive so X11 can serve later paste requests.
pub(crate) fn spawn(app: AppHandle, store: Arc<Mutex<HistoryStore>>, rx: mpsc::Receiver<Write>) {
    std::thread::spawn(move || {
        let result = run(&app, &store, rx);
        if let Err(error) = result {
            eprintln!("yoink: clipboard monitoring stopped: {error}");
        }
    });
}

fn run(
    app: &AppHandle,
    store: &Mutex<HistoryStore>,
    rx: mpsc::Receiver<Write>,
) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|e| e.to_string())?;
    let mut watcher = Watcher::new().map_err(|e| e.to_string())?;
    let mut processed = None;
    let mut failed_revision = None;
    let mut retry_delay = Duration::from_secs(1);
    let mut next_retry = Instant::now();
    loop {
        match rx.recv_timeout(POLL_INTERVAL) {
            Ok(write) => {
                let result = match write.content {
                    Content::Text(text) => clipboard.set_text(text),
                    Content::Image {
                        rgba,
                        width,
                        height,
                    } => clipboard.set_image(arboard::ImageData {
                        width: width as usize,
                        height: height as usize,
                        bytes: Cow::Owned(rgba),
                    }),
                }
                .map_err(|e| e.to_string());
                let _ = write.reply.send(result);
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Ok(()),
        }
        let revision = watcher.revision().map_err(|e| e.to_string())?;
        if failed_revision != Some(revision) {
            retry_delay = Duration::from_secs(1);
        }
        if processed == Some(revision)
            || (failed_revision == Some(revision) && Instant::now() < next_retry)
        {
            continue;
        }
        match capture(&mut clipboard, &mut watcher, revision, store) {
            Ok(changed) => {
                processed = Some(revision);
                failed_revision = None;
                if changed {
                    let _ = app.emit("history-changed", ());
                }
            }
            Err(error) => {
                if failed_revision != Some(revision) {
                    eprintln!("yoink: cannot capture clipboard: {error}; retrying with backoff");
                }
                failed_revision = Some(revision);
                next_retry = Instant::now() + retry_delay;
                retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
            }
        }
    }
}

fn capture(
    clipboard: &mut arboard::Clipboard,
    watcher: &mut Watcher,
    revision: u64,
    store: &Mutex<HistoryStore>,
) -> Result<bool, String> {
    if watcher.is_sensitive().map_err(|e| e.to_string())? {
        return Ok(false);
    }
    if let Ok(text) = clipboard.get_text() {
        if text.len() > MAX_TEXT_BYTES {
            eprintln!("yoink: skipping text over 1 MiB");
            return Ok(false);
        }
        if watcher.revision().map_err(|e| e.to_string())? != revision {
            return Ok(false);
        }
        return store
            .lock()
            .unwrap()
            .record_text(&text, crate::now_ms())
            .map(|item| item.is_some())
            .map_err(|e| e.to_string());
    }
    let image = match clipboard.get_image() {
        Ok(image) => image,
        Err(arboard::Error::ContentNotAvailable) => return Ok(false),
        Err(error) => return Err(error.to_string()),
    };
    if !valid_image_size(image.width, image.height, image.bytes.len()) {
        eprintln!("yoink: skipping image over the decoded size limit");
        return Ok(false);
    }
    let (width, height) = (image.width as u32, image.height as u32);
    let png = encode_png(&image.bytes, width, height)?;
    if png.len() > MAX_IMAGE_PNG_BYTES {
        eprintln!("yoink: skipping image over 10 MiB PNG");
        return Ok(false);
    }
    let full = RgbaImage::from_raw(width, height, image.bytes.into_owned())
        .ok_or("invalid image dimensions")?;
    let thumb = image::DynamicImage::ImageRgba8(full)
        .thumbnail(400, 240)
        .to_rgba8();
    let thumb = encode_png(thumb.as_raw(), thumb.width(), thumb.height())?;
    if watcher.revision().map_err(|e| e.to_string())? != revision {
        return Ok(false);
    }
    store
        .lock()
        .unwrap()
        .record_image(&png, &thumb, width, height, crate::now_ms())
        .map(|item| item.is_some())
        .map_err(|e| e.to_string())
}

type XResult<T> = Result<T, Box<dyn std::error::Error>>;

struct Watcher {
    conn: RustConnection,
    root: u32,
    clipboard: Atom,
    targets: Atom,
    secret: Atom,
    serial: u64,
}

impl Watcher {
    fn new() -> XResult<Self> {
        let (conn, screen) = x11rb::connect(None)?;
        conn.xfixes_query_version(1, 0)?.reply()?;
        let root = conn.setup().roots[screen].root;
        let clipboard = conn.intern_atom(false, b"CLIPBOARD")?.reply()?.atom;
        let targets = conn.intern_atom(false, b"TARGETS")?.reply()?.atom;
        let secret = conn
            .intern_atom(false, b"x-kde-passwordManagerHint")?
            .reply()?
            .atom;
        conn.xfixes_select_selection_input(
            root,
            clipboard,
            SelectionEventMask::SET_SELECTION_OWNER
                | SelectionEventMask::SELECTION_WINDOW_DESTROY
                | SelectionEventMask::SELECTION_CLIENT_CLOSE,
        )?
        .check()?;
        conn.flush()?;
        Ok(Self {
            conn,
            root,
            clipboard,
            targets,
            secret,
            serial: 0,
        })
    }

    fn revision(&mut self) -> XResult<u64> {
        // A round trip drains ownership events queued while arboard fetched the data.
        self.conn.get_input_focus()?.reply()?;
        while let Some(event) = self.conn.poll_for_event()? {
            self.observe(&event);
        }
        Ok(self.serial)
    }

    fn observe(&mut self, event: &Event) {
        if let Event::XfixesSelectionNotify(event) = event {
            if event.selection == self.clipboard {
                self.serial = self.serial.wrapping_add(1);
            }
        }
    }

    fn is_sensitive(&mut self) -> XResult<bool> {
        let window = self.conn.generate_id()?;
        self.conn
            .create_window(
                0,
                window,
                self.root,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_ONLY,
                0,
                &CreateWindowAux::new(),
            )?
            .check()?;
        let result = (|| {
            if self
                .conn
                .get_selection_owner(self.clipboard)?
                .reply()?
                .owner
                == x11rb::NONE
            {
                return Ok(false);
            }
            let targets = self
                .read_property(window, self.targets, 4096)?
                .ok_or("clipboard owner did not list its formats")?;
            if targets.bytes_after > 0 || targets.format != 32 {
                return Err("clipboard format list is invalid; skipping capture".into());
            }
            if !targets
                .value32()
                .is_some_and(|mut atoms| atoms.any(|atom| atom == self.secret))
            {
                return Ok(false);
            }
            let hint = self
                .read_property(window, self.secret, 16)?
                .ok_or("clipboard owner did not supply its advertised secret marker")?;
            Ok(hint.bytes_after > 0
                || hint.format != 8
                || hint.value.strip_suffix(&[0]).unwrap_or(&hint.value) == b"secret")
        })();
        self.conn.destroy_window(window)?.check()?;
        result
    }

    fn read_property(
        &mut self,
        window: u32,
        target: Atom,
        length: u32,
    ) -> XResult<Option<x11rb::protocol::xproto::GetPropertyReply>> {
        self.conn
            .convert_selection(
                window,
                self.clipboard,
                target,
                self.secret,
                x11rb::CURRENT_TIME,
            )?
            .check()?;
        self.conn.flush()?;
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            while let Some(event) = self.conn.poll_for_event()? {
                self.observe(&event);
                if let Event::SelectionNotify(event) = event {
                    if event.requestor != window || event.target != target {
                        continue;
                    }
                    if event.property == x11rb::NONE {
                        return Ok(None);
                    }
                    let property = self
                        .conn
                        .get_property(true, window, self.secret, AtomEnum::ANY, 0, length)?
                        .reply()?;
                    return Ok(Some(property));
                }
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        Err("clipboard format request timed out; skipping capture".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires an isolated X11 display"]
    fn x11_recopies_and_password_exclusions() {
        use arboard::SetExtLinux;
        let mut clipboard = arboard::Clipboard::new().unwrap();
        let mut watcher = Watcher::new().unwrap();
        let store = Mutex::new(HistoryStore::open_in_memory().unwrap());
        clipboard.set_text("synthetic recopy").unwrap();
        let first_revision = watcher.revision().unwrap();
        assert!(capture(&mut clipboard, &mut watcher, first_revision, &store).unwrap());
        let id = store.lock().unwrap().list("", 1).unwrap()[0].id;
        store.lock().unwrap().remove(id).unwrap();
        assert_eq!(watcher.revision().unwrap(), first_revision);
        clipboard.set_text("synthetic recopy").unwrap();
        let second_revision = watcher.revision().unwrap();
        assert_ne!(second_revision, first_revision);
        assert!(capture(&mut clipboard, &mut watcher, second_revision, &store).unwrap());
        store.lock().unwrap().clear().unwrap();
        clipboard
            .set()
            .exclude_from_history()
            .text("synthetic secret")
            .unwrap();
        let revision = watcher.revision().unwrap();
        assert!(!capture(&mut clipboard, &mut watcher, revision, &store).unwrap());
        assert!(store.lock().unwrap().is_empty().unwrap());
        clipboard.set_text("x".repeat(MAX_TEXT_BYTES + 1)).unwrap();
        let revision = watcher.revision().unwrap();
        assert!(!capture(&mut clipboard, &mut watcher, revision, &store).unwrap());
        assert!(store.lock().unwrap().is_empty().unwrap());
        clipboard.set_text("ordinary text after secret").unwrap();
        let revision = watcher.revision().unwrap();
        assert!(capture(&mut clipboard, &mut watcher, revision, &store).unwrap());
        assert_eq!(
            store.lock().unwrap().list("", 1).unwrap()[0].text,
            "ordinary text after secret"
        );
        clipboard
            .set_image(arboard::ImageData {
                width: 2,
                height: 2,
                bytes: Cow::Owned(vec![255; 16]),
            })
            .unwrap();
        let revision = watcher.revision().unwrap();
        assert!(capture(&mut clipboard, &mut watcher, revision, &store).unwrap());
        assert_eq!(
            store.lock().unwrap().list("", 1).unwrap()[0].kind,
            yoink_core::ItemKind::Image
        );
    }

    #[test]
    #[ignore = "requires an isolated X11 display and xclip"]
    fn x11_external_image_capture() {
        assert!(std::process::Command::new("xclip")
            .args([
                "-selection",
                "clipboard",
                "-target",
                "image/png",
                "-i",
                concat!(env!("CARGO_MANIFEST_DIR"), "/icons/128x128.png"),
            ])
            .status()
            .unwrap()
            .success());
        let mut clipboard = arboard::Clipboard::new().unwrap();
        let mut watcher = Watcher::new().unwrap();
        let store = Mutex::new(HistoryStore::open_in_memory().unwrap());
        let revision = watcher.revision().unwrap();
        assert!(!watcher.is_sensitive().unwrap());
        assert!(capture(&mut clipboard, &mut watcher, revision, &store).unwrap());
        assert_eq!(
            store.lock().unwrap().list("", 1).unwrap()[0].kind,
            yoink_core::ItemKind::Image
        );
    }

    #[test]
    fn rejects_large_or_inconsistent_bitmaps_before_encoding() {
        assert!(valid_image_size(3840, 2160, 3840 * 2160 * 4));
        assert!(!valid_image_size(8192, 8192, 8192 * 8192 * 4));
        assert!(!valid_image_size(usize::MAX, 4, 16));
        assert!(!valid_image_size(0, 10, 0));
        assert!(!valid_image_size(10, 10, 399));
        let oversized = encode_png(&vec![0; 8193 * 4], 8193, 1).unwrap();
        assert!(decode_image(oversized).is_err());
    }
}
