//! One editor per install.
//!
//! Double-clicking a `.md` file on the desktop starts the program again, and a
//! second window over the first one is rarely what was meant: the file belongs in
//! the editor that is already open, with its tabs, its undo history and the size
//! and place the user left it in. So a launch first looks for a running instance
//! for the same data folder and hands it the file names; only when there is none
//! does it become the instance itself.
//!
//! The channel is a loopback TCP socket whose port and a per-run token are kept in
//! `instance.lock` next to `settings.conf`:
//!
//! ```text
//! launch A: create_new(instance.lock) -> OK   -> listen on 127.0.0.1:<port>
//!                                             -> write "<port> <token>"
//! launch B: create_new(instance.lock) -> EEXIST
//!           read "<port> <token>", connect, send "<token>\0<path>\0<path>\0"
//!                                             -> the running instance opens them
//! launch C after A was killed: connect refused -> the lock is stale, delete it,
//!           try again: C becomes the instance
//! ```
//!
//! No dependency beyond `std`, nothing listening on anything but the loopback
//! interface, and a token so another local user cannot push paths into the editor.
//! When loopback is unavailable (a sandbox with no network, a locked-down
//! container) the call degrades to [`Claim::Solo`] and the editor simply runs
//! alone, exactly as it did before any of this existed.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// How long a launch waits for a running instance before deciding there is none.
const CONNECT_TIMEOUT: Duration = Duration::from_millis(500);
/// A second instance only ever sends file names; anything bigger is not us.
const MAX_MESSAGE: u64 = 1 << 20;

/// What a launch learned about the folder it is starting in.
pub enum Claim {
    /// Nobody else had it: this process is the instance, and `Receiver` will
    /// deliver the paths every later launch hands over.
    Primary(Claimed),
    /// A running instance took the files; this process should exit at once,
    /// without loading settings or opening a window.
    Sent,
    /// No way to talk to another instance (no loopback, a read-only folder): run
    /// alone.
    Solo,
}

/// Paths a later launch handed over, and a way to be woken when one arrives.
///
/// The event loop is asleep most of the time (`ControlFlow::WaitUntil`, seconds
/// away); a file dropped in by a second launch has to wake it, and the wake-up
/// mechanism (`EventLoopProxy`) belongs to the window layer, not here. So this is
/// a queue plus a condition variable: one thread waits on it, the loop drains it.
struct Shared {
    /// One entry per launch that reached us - empty when a launch only asked the
    /// running instance to come to the front.
    queue: Mutex<Vec<Vec<PathBuf>>>,
    arrived: Condvar,
    open: Mutex<bool>,
}

/// The primary instance's end of the handoff.
pub struct Claimed {
    shared: Arc<Shared>,
    lock: PathBuf,
}

impl Claimed {
    /// Batches forwarded since the last call, one per launch (never blocks).
    pub fn take(&self) -> Vec<Vec<PathBuf>> {
        let mut queue = self.shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::take(&mut *queue)
    }

    /// Block until something has arrived (or the instance is released).
    ///
    /// Peeks rather than drains: the event loop is what takes the paths out.
    pub fn wait_any(&self) -> bool {
        let mut queue = self.shared.queue.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if !queue.is_empty() {
                return true;
            }
            let open = self.shared.open.lock().unwrap_or_else(|e| e.into_inner());
            if !*open {
                return false;
            }
            drop(open);
            let (next, _) = self
                .shared
                .arrived
                .wait_timeout(queue, Duration::from_millis(500))
                .unwrap_or_else(|e| e.into_inner());
            queue = next;
        }
    }

    /// Give the folder up on the way out, so the next launch starts cleanly
    /// instead of waiting for a connect timeout against a dead port.
    pub fn release(&self) {
        let _ = std::fs::remove_file(&self.lock);
        *self.shared.open.lock().unwrap_or_else(|e| e.into_inner()) = false;
        self.shared.arrived.notify_all();
    }
}

fn lock_file(dir: &Path) -> PathBuf {
    dir.join("instance.lock")
}

/// Claim `dir` for this process, or hand `files` to whoever already has it.
pub fn claim(dir: &Path, files: &[PathBuf]) -> Claim {
    let _ = std::fs::create_dir_all(dir);
    let lock = lock_file(dir);
    // Two launches at the same instant can both find the lock stale; a couple of
    // rounds are enough for one of them to create it and the other to see it.
    for _ in 0..4 {
        match std::fs::OpenOptions::new().write(true).create_new(true).open(&lock) {
            Ok(mut file) => {
                // We are the instance. Bind first: a port is needed before the lock
                // can describe where to find us.
                let Ok(listener) = TcpListener::bind(("127.0.0.1", 0)) else {
                    // No loopback at all (a sandbox). Leave no lock behind that
                    // would make the *next* launch wait for a port that is not
                    // there.
                    let _ = std::fs::remove_file(&lock);
                    return Claim::Solo;
                };
                let Ok(addr) = listener.local_addr() else {
                    let _ = std::fs::remove_file(&lock);
                    return Claim::Solo;
                };
                let token = make_token();
                let _ = write!(file, "{} {}", addr.port(), token);
                let _ = file.flush();
                drop(file);
                log::info!("instance for {}: listening on {}", dir.display(), addr);
                let shared = Arc::new(Shared {
                    queue: Mutex::new(Vec::new()),
                    arrived: Condvar::new(),
                    open: Mutex::new(true),
                });
                serve(listener, token, shared.clone());
                return Claim::Primary(Claimed { shared, lock });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if let Some((port, token)) = read_lock(&lock) {
                    if send(port, &token, files) {
                        log::info!("handed {} path(s) to the running instance", files.len());
                        return Claim::Sent;
                    }
                }
                // The lock is stale: its owner died without cleaning up.
                log::warn!("stale instance lock; taking over");
                let _ = std::fs::remove_file(&lock);
            }
            Err(e) => {
                log::warn!("cannot create the instance lock: {e}");
                return Claim::Solo;
            }
        }
    }
    Claim::Solo
}

fn read_lock(path: &Path) -> Option<(u16, String)> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut parts = text.split_whitespace();
    let port = parts.next()?.parse().ok()?;
    let token = parts.next()?.to_string();
    Some((port, token))
}

/// Tell the instance at `port` about `files`. `true` when it answered.
fn send(port: u16, token: &str, files: &[PathBuf]) -> bool {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let Ok(mut stream) = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) else {
        return false;
    };
    let _ = stream.set_write_timeout(Some(CONNECT_TIMEOUT));
    // `\0` separates the fields: no path on any platform may contain one.
    let mut message = String::with_capacity(64);
    message.push_str(token);
    for path in files {
        message.push('\0');
        message.push_str(&path.to_string_lossy());
    }
    message.push('\0');
    stream.write_all(message.as_bytes()).is_ok() && stream.flush().is_ok()
}

/// Accept connections for the life of the process, queueing validated batches.
fn serve(listener: TcpListener, token: String, shared: Arc<Shared>) {
    let spawned = std::thread::Builder::new()
        .name("bare-md-handoff".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut buf = Vec::new();
                let mut limited = Read::take(&mut stream, MAX_MESSAGE);
                if limited.read_to_end(&mut buf).is_err() {
                    continue;
                }
                let text = String::from_utf8_lossy(&buf).into_owned();
                let mut parts = text.split('\0');
                if parts.next() != Some(token.as_str()) {
                    // Not our launcher: ignore it and keep listening.
                    continue;
                }
                let files: Vec<PathBuf> =
                    parts.filter(|p| !p.is_empty()).map(PathBuf::from).collect();
                log::info!("handoff: {} path(s)", files.len());
                shared
                    .queue
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(files);
                shared.arrived.notify_all();
            }
        });
    if spawned.is_err() {
        log::warn!("cannot start the handoff thread; this instance will not take files");
    }
}

/// A per-run secret, so another local user cannot push paths into the editor.
///
/// The lock file is only readable by its owner in a normal install, but a token
/// costs nothing and closes the hole on a shared folder.
fn make_token() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let pid = std::process::id() as u64;
    // xorshift: enough to make the token unguessable, and no dependency.
    let mut x = nanos ^ (pid << 32) ^ 0x9e37_79b9_7f4a_7c15;
    x ^= x << 13;
    x ^= x >> 7;
    x ^= x << 17;
    format!("{x:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bare-md-single-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn the_second_launch_hands_its_files_to_the_first() {
        let dir = temp("handoff");
        let Claim::Primary(first) = claim(&dir, &[]) else {
            panic!("the first claim should have become the instance");
        };
        let wanted = vec![dir.join("a.md"), dir.join("b.md")];
        assert!(matches!(claim(&dir, &wanted), Claim::Sent), "the second launch did not forward");
        // …and the first instance receives exactly those paths.
        let mut got: Vec<Vec<PathBuf>> = Vec::new();
        for _ in 0..50 {
            got = first.take();
            if !got.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(got, vec![wanted], "the running instance did not receive the paths");
        first.release();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A launch with no file on its command line - the taskbar icon clicked twice -
    /// still reaches the running instance, as an empty batch, so the window can be
    /// raised instead of a second copy appearing.
    #[test]
    fn a_launch_with_no_files_still_reaches_the_running_instance() {
        let dir = temp("raise");
        let Claim::Primary(first) = claim(&dir, &[]) else { panic!("no instance") };
        assert!(matches!(claim(&dir, &[]), Claim::Sent), "a bare launch was not forwarded");
        let mut got: Vec<Vec<PathBuf>> = Vec::new();
        for _ in 0..50 {
            got = first.take();
            if !got.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(got, vec![Vec::<PathBuf>::new()], "the bare launch did not arrive");
        first.release();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_stale_lock_is_taken_over() {
        let dir = temp("stale");
        // An instance that died: the file is there, nothing is listening.
        std::fs::write(lock_file(&dir), "1 deadbeefdeadbeef").unwrap();
        let files = vec![dir.join("c.md")];
        match claim(&dir, &files) {
            Claim::Primary(claimed) => claimed.release(),
            Claim::Sent => panic!("forwarded to a port that is not listening"),
            Claim::Solo => panic!("gave up instead of taking the stale lock over"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_wrong_token_is_ignored() {
        let dir = temp("token");
        let Claim::Primary(claimed) = claim(&dir, &[]) else {
            panic!("no instance");
        };
        let (port, _) = read_lock(&lock_file(&dir)).expect("no lock file written");
        // The right port, the wrong secret: the write may succeed, but the
        // editor must never see the path.
        let _ = send(port, "not-the-token", &[dir.join("evil.md")]);
        let mut got: Vec<Vec<PathBuf>> = Vec::new();
        for _ in 0..10 {
            got = claimed.take();
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(got.is_empty(), "a message with the wrong token was accepted: {got:?}");
        claimed.release();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn releasing_the_instance_lets_the_next_launch_have_it() {
        let dir = temp("release");
        let Claim::Primary(first) = claim(&dir, &[]) else { panic!("no instance") };
        first.release();
        assert!(!lock_file(&dir).exists(), "the lock file survived release");
        match claim(&dir, &[]) {
            Claim::Primary(second) => second.release(),
            _ => panic!("the second launch could not become the instance"),
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
