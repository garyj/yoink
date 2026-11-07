//! Hands a request to the running instance over a Unix socket. The client
//! side runs before any UI code loads, so a summon costs one connect.

use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
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

/// The directory for the socket and its lock file.
pub fn runtime_dir(fallback_dir: &Path) -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_dir())
        .unwrap_or_else(|| fallback_dir.to_path_buf())
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

/// Held for the life of the running instance. Only the holder touches the
/// socket file, so two launches cannot both judge it stale and race to
/// replace it.
pub struct InstanceLock {
    _file: File,
}

/// Returns `None` when another instance holds the lock.
pub fn lock(path: &Path) -> io::Result<Option<InstanceLock>> {
    let file = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .mode(0o600)
        .open(path)?;
    // SAFETY: flock only uses the descriptor, which `file` keeps open.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(Some(InstanceLock { _file: file }));
    }
    let error = io::Error::last_os_error();
    if error.kind() == io::ErrorKind::WouldBlock {
        Ok(None)
    } else {
        Err(error)
    }
}

/// Binds the socket, replacing a file a crashed instance left behind. Call
/// only while holding the [`InstanceLock`].
pub fn bind(path: &Path, _lock: &InstanceLock) -> io::Result<UnixListener> {
    match std::fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
        _ => {}
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
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
    fn one_instance_owns_the_socket_and_receives_requests() {
        let dir = tempfile::tempdir().unwrap();
        let socket = dir.path().join("yoink-gpui.sock");
        let lock_path = dir.path().join("yoink-gpui.lock");
        assert!(!send(&socket, Request::Toggle));

        let lock = lock(&lock_path)
            .unwrap()
            .expect("the first launch takes the lock");
        assert!(
            super::lock(&lock_path).unwrap().is_none(),
            "a second launch must not take over"
        );
        // A socket file with no listener, as a crash leaves behind.
        drop(UnixListener::bind(&socket).unwrap());
        let listener = bind(&socket, &lock).unwrap();

        let (tx, rx) = mpsc::channel();
        serve(listener, move |request| tx.send(request).unwrap());
        for request in [Request::Toggle, Request::Show, Request::Hide, Request::Quit] {
            assert!(send(&socket, request));
            assert_eq!(rx.recv_timeout(Duration::from_secs(2)).unwrap(), request);
        }
        drop(lock);
        assert!(
            super::lock(&lock_path).unwrap().is_some(),
            "exit releases the lock"
        );
    }
}
