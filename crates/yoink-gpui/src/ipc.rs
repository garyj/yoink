//! Hands a request to the running instance over a Unix socket. The client
//! side runs before any UI code loads, so a summon costs one connect.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// Asks only whether an instance is running.
    Ping,
    Toggle,
    Show,
    Hide,
    Quit,
}

impl Request {
    fn as_str(self) -> &'static str {
        match self {
            Request::Ping => "ping",
            Request::Toggle => "toggle",
            Request::Show => "show",
            Request::Hide => "hide",
            Request::Quit => "quit",
        }
    }

    fn parse(line: &str) -> Option<Self> {
        match line.trim() {
            "ping" => Some(Request::Ping),
            "toggle" => Some(Request::Toggle),
            "show" => Some(Request::Show),
            "hide" => Some(Request::Hide),
            "quit" => Some(Request::Quit),
            _ => None,
        }
    }
}

pub fn socket_path(fallback_dir: &Path) -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(|| fallback_dir.to_path_buf())
        .join("yoink-gpui.sock")
}

/// Returns `false` when no instance is listening.
pub fn send(path: &Path, request: Request) -> bool {
    let Ok(mut stream) = UnixStream::connect(path) else {
        return false;
    };
    stream
        .write_all(format!("{}\n", request.as_str()).as_bytes())
        .is_ok()
}

pub enum Bind {
    Listening(UnixListener),
    /// Another instance won the race to bind the socket.
    AlreadyRunning,
}

pub fn bind(path: &Path) -> io::Result<Bind> {
    let listener = match UnixListener::bind(path) {
        Ok(listener) => listener,
        Err(error) if error.kind() == io::ErrorKind::AddrInUse => {
            if UnixStream::connect(path).is_ok() {
                return Ok(Bind::AlreadyRunning);
            }
            // Nothing accepts on the socket, so a crashed instance left it behind.
            std::fs::remove_file(path)?;
            UnixListener::bind(path)?
        }
        Err(error) => return Err(error),
    };
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(Bind::Listening(listener))
}

/// Calls `handle` on a background thread for each request received.
pub fn serve(listener: UnixListener, handle: impl Fn(Request) + Send + 'static) {
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut line = String::new();
            if BufReader::new(stream).read_line(&mut line).is_ok()
                && let Some(request) = Request::parse(&line)
            {
                handle(request);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn requests_reach_the_running_instance_and_stale_sockets_are_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("yoink-gpui.sock");
        assert!(!send(&path, Request::Toggle));

        // A socket file with no listener, as a crash leaves behind.
        drop(UnixListener::bind(&path).unwrap());
        let Bind::Listening(listener) = bind(&path).unwrap() else {
            panic!("a stale socket must not count as a running instance");
        };
        assert!(matches!(bind(&path).unwrap(), Bind::AlreadyRunning));

        let (tx, rx) = mpsc::channel();
        serve(listener, move |request| tx.send(request).unwrap());
        for request in [Request::Toggle, Request::Show, Request::Hide, Request::Quit] {
            assert!(send(&path, request));
            assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), request);
        }
    }
}
