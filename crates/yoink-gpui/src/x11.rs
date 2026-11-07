//! Window placement and activation through a private X11 connection.
//!
//! Cinnamon with `focus-new-windows 'smart'` gives focus to a newly mapped
//! window only when its `_NET_WM_USER_TIME` is newer than the focused
//! window's. An activation request with `CURRENT_TIME`, or one sent before
//! the window is viewable, is often refused and leaves the window mapped
//! behind the terminal.

use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::properties::{WmSizeHints, WmSizeHintsSpecification};
use x11rb::protocol::Event;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ConfigureWindowAux, ConnectionExt as _, CreateWindowAux,
    EventMask, MapState, PropMode, StackMode, UnmapNotifyEvent, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

pub fn window_id(window: &gpui_kit::Window) -> Option<Window> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Xcb(handle) => Some(handle.window.get()),
        RawWindowHandle::Xlib(handle) => Some(handle.window as Window),
        _ => None,
    }
}

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

const VIEWABLE_TIMEOUT: Duration = Duration::from_millis(500);

/// The `_NET_WM_DESKTOP` value of a window shown on every workspace.
const ALL_DESKTOPS: u32 = u32::MAX;

pub struct X11 {
    conn: RustConnection,
    root: Window,
    clock: Window,
    net_active_window: Atom,
    net_current_desktop: Atom,
    net_wm_desktop: Atom,
    net_wm_user_time: Atom,
    yoink_clock: Atom,
}

impl X11 {
    pub fn connect() -> Result<Self> {
        let (conn, screen) = x11rb::connect(None)?;
        let root = conn.setup().roots[screen].root;
        let intern =
            |name: &[u8]| -> Result<Atom> { Ok(conn.intern_atom(false, name)?.reply()?.atom) };
        let net_active_window = intern(b"_NET_ACTIVE_WINDOW")?;
        let net_current_desktop = intern(b"_NET_CURRENT_DESKTOP")?;
        let net_wm_desktop = intern(b"_NET_WM_DESKTOP")?;
        let net_wm_user_time = intern(b"_NET_WM_USER_TIME")?;
        let yoink_clock = intern(b"_YOINK_CLOCK")?;
        let clock = conn.generate_id()?;
        conn.create_window(
            0,
            clock,
            root,
            -1,
            -1,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            0,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )?
        .check()?;
        Ok(Self {
            conn,
            root,
            clock,
            net_active_window,
            net_current_desktop,
            net_wm_desktop,
            net_wm_user_time,
            yoink_clock,
        })
    }

    /// The X server's current time, read from the timestamp of a property
    /// change on a private window. It is newer than any key press the user
    /// has made, which the window manager needs to grant focus.
    fn server_time(&self) -> Result<u32> {
        self.conn.change_property32(
            PropMode::REPLACE,
            self.clock,
            self.yoink_clock,
            AtomEnum::CARDINAL,
            &[0],
        )?;
        self.conn.flush()?;
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            match self.conn.poll_for_event()? {
                Some(Event::PropertyNotify(event)) if event.window == self.clock => {
                    return Ok(event.time);
                }
                Some(_) => {}
                None if Instant::now() > deadline => return Err("no X server timestamp".into()),
                None => std::thread::sleep(Duration::from_micros(200)),
            }
        }
    }

    pub fn is_viewable(&self, window: Window) -> bool {
        self.conn
            .get_window_attributes(window)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .is_some_and(|attributes| attributes.map_state == MapState::VIEWABLE)
    }

    /// Maps `window` centered on the monitor under the pointer and asks the
    /// window manager to focus it.
    pub fn show(&self, window: Window, width: u32, height: u32) -> Result<()> {
        let time = self.server_time()?;
        self.conn.change_property32(
            PropMode::REPLACE,
            window,
            self.net_wm_user_time,
            AtomEnum::CARDINAL,
            &[time],
        )?;
        self.place(window, width, height)?;
        self.conn.map_window(window)?;
        self.conn.flush()?;
        if !self.wait_viewable(window) {
            return Err("window did not become viewable".into());
        }
        self.activate(window, time)
    }

    fn wait_viewable(&self, window: Window) -> bool {
        let deadline = Instant::now() + VIEWABLE_TIMEOUT;
        while !self.is_viewable(window) {
            if Instant::now() > deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        true
    }

    /// Raises and focuses an already mapped window.
    pub fn activate(&self, window: Window, time: u32) -> Result<()> {
        // Source indication 2 marks a user action, as taskbars and pagers send.
        let event = ClientMessageEvent::new(32, window, self.net_active_window, [2, time, 0, 0, 0]);
        self.conn.send_event(
            false,
            self.root,
            EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT,
            event,
        )?;
        self.conn.configure_window(
            window,
            &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE),
        )?;
        self.conn.flush()?;
        Ok(())
    }

    /// Focuses an already shown window, first moving it to the current
    /// workspace as `wmctrl -R` does.
    pub fn summon(&self, window: Window, width: u32, height: u32) -> Result<()> {
        if self.move_to_current_desktop(window)? {
            self.place(window, width, height)?;
        }
        let time = self.server_time()?;
        self.activate(window, time)
    }

    /// Cinnamon keeps a window on another workspace mapped and viewable, and
    /// activating it switches to that workspace. Returns whether it moved.
    fn move_to_current_desktop(&self, window: Window) -> Result<bool> {
        let (Some(current), Some(desktop)) = (
            self.cardinal(self.root, self.net_current_desktop)?,
            self.cardinal(window, self.net_wm_desktop)?,
        ) else {
            return Ok(false);
        };
        if desktop == current || desktop == ALL_DESKTOPS {
            return Ok(false);
        }
        let event = ClientMessageEvent::new(32, window, self.net_wm_desktop, [current, 2, 0, 0, 0]);
        self.conn.send_event(
            false,
            self.root,
            EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT,
            event,
        )?;
        self.conn.flush()?;
        let deadline = Instant::now() + VIEWABLE_TIMEOUT;
        while self.cardinal(window, self.net_wm_desktop)? != Some(current) {
            if Instant::now() > deadline {
                return Err("window did not move to the current workspace".into());
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        Ok(true)
    }

    fn cardinal(&self, window: Window, property: Atom) -> Result<Option<u32>> {
        let reply = self
            .conn
            .get_property(false, window, property, AtomEnum::CARDINAL, 0, 1)?
            .reply()?;
        Ok(reply.value32().and_then(|mut values| values.next()))
    }

    /// Withdraws `window` as ICCCM 4.1.4 describes: unmap it, then send a
    /// synthetic UnmapNotify so a reparenting window manager stops managing it.
    pub fn hide(&self, window: Window) -> Result<()> {
        // GPUI maps every new window. Unmapping before the window manager
        // has handled that map would let the map win.
        self.wait_viewable(window);
        self.conn.unmap_window(window)?;
        let event = UnmapNotifyEvent {
            response_type: x11rb::protocol::xproto::UNMAP_NOTIFY_EVENT,
            sequence: 0,
            event: self.root,
            window,
            from_configure: false,
        };
        self.conn.send_event(
            false,
            self.root,
            EventMask::SUBSTRUCTURE_NOTIFY | EventMask::SUBSTRUCTURE_REDIRECT,
            event,
        )?;
        self.conn.flush()?;
        Ok(())
    }

    /// Centers `window` on the monitor under the pointer.
    fn place(&self, window: Window, width: u32, height: u32) -> Result<()> {
        let Some((x, y)) = self.centered_position(width, height) else {
            return Ok(());
        };
        let mut hints = WmSizeHints::get_normal_hints(&self.conn, window)?
            .reply()?
            .unwrap_or_default();
        hints.position = Some((WmSizeHintsSpecification::UserSpecified, x, y));
        hints.set_normal_hints(&self.conn, window)?;
        self.conn
            .configure_window(window, &ConfigureWindowAux::new().x(x).y(y))?;
        Ok(())
    }

    fn centered_position(&self, width: u32, height: u32) -> Option<(i32, i32)> {
        let pointer = self.conn.query_pointer(self.root).ok()?.reply().ok()?;
        let (px, py) = (i32::from(pointer.root_x), i32::from(pointer.root_y));
        let monitors = self
            .conn
            .randr_get_monitors(self.root, true)
            .ok()?
            .reply()
            .ok()?;
        let monitor = monitors
            .monitors
            .iter()
            .find(|m| {
                let (x, y) = (i32::from(m.x), i32::from(m.y));
                px >= x && px < x + i32::from(m.width) && py >= y && py < y + i32::from(m.height)
            })
            .or_else(|| monitors.monitors.iter().find(|m| m.primary))
            .or(monitors.monitors.first())?;
        let x = i32::from(monitor.x) + (i32::from(monitor.width) - width as i32) / 2;
        let y = i32::from(monitor.y) + (i32::from(monitor.height) - height as i32) * 2 / 5;
        Some((x, y))
    }
}

pub enum Command {
    Show {
        window: Window,
        width: u32,
        height: u32,
    },
    Hide {
        window: Window,
    },
}

/// Runs X11 requests on their own thread, so waiting for the window manager
/// never blocks the frame GPUI draws when the window maps.
pub fn spawn_worker() -> std::sync::mpsc::Sender<Command> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let x11 = match X11::connect() {
            Ok(x11) => x11,
            Err(error) => {
                eprintln!("yoink: cannot connect to X11: {error}");
                return;
            }
        };
        // The window opens off screen, so the first show must place it.
        let mut placed = false;
        for command in rx {
            let result = match command {
                Command::Show {
                    window,
                    width,
                    height,
                } if placed && x11.is_viewable(window) => x11.summon(window, width, height),
                Command::Show {
                    window,
                    width,
                    height,
                } => {
                    placed = true;
                    x11.show(window, width, height)
                }
                Command::Hide { window } => x11.hide(window),
            };
            if let Err(error) = result {
                eprintln!("yoink: window request failed: {error}");
            }
        }
    });
    tx
}
