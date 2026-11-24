//! One window at a time (feature `single-instance`): a second launch brings the running
//! window to the front and exits, so two copies never share the index or the embedded
//! database. Standard library only: an exclusive lock on `<data>/instance.lock`, and the
//! first instance listens on a loopback port (written beside the lock) for "focus".
//!
//! # Example
//!
//! What a second launch does, with the standard library only: the lock is already held, so
//! it knocks on the port the first instance wrote and exits.
//!
//! ```
//! let dir = std::env::temp_dir().join(format!("single-doc-{}", std::process::id()));
//! std::fs::create_dir_all(&dir)?;
//! let first = std::fs::File::create(dir.join("instance.lock"))?;
//! first.try_lock().expect("nobody else holds it");
//! let second = std::fs::File::open(dir.join("instance.lock"))?;
//! assert!(second.try_lock().is_err()); // a second launch: knock and exit
//! # let _ = std::fs::remove_dir_all(&dir);
//! # Ok::<(), std::io::Error>(())
//! ```

use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::Path;
use std::time::Duration;

const KNOCK: &[u8] = b"focus";

/// Keeps the lock for the life of the app (Tauri state).
pub struct Held(#[allow(dead_code)] pub std::fs::File);

/// Either we are the only instance (keep the returned lock and listener for the life of
/// the process), or another one was asked to show itself and this one should exit.
pub enum Instance {
    /// This is the only instance: keep the lock file and the listener.
    First(std::fs::File, TcpListener),
    /// Another instance was asked to come to the front; exit.
    Second,
}

/// Take the instance lock in `data_dir`, or knock on the instance that holds it.
pub fn claim(data_dir: &Path) -> std::io::Result<Instance> {
    std::fs::create_dir_all(data_dir)?;
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(data_dir.join("instance.lock"))?;
    let port_file = data_dir.join("instance.port");
    match lock.try_lock() {
        Ok(()) => {
            let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
            std::fs::write(&port_file, listener.local_addr()?.port().to_string())?;
            Ok(Instance::First(lock, listener))
        }
        Err(std::fs::TryLockError::WouldBlock) => {
            let port: u16 = std::fs::read_to_string(&port_file).ok().and_then(|p| p.trim().parse().ok()).unwrap_or(0);
            if let Ok(mut s) = TcpStream::connect_timeout(&(Ipv4Addr::LOCALHOST, port).into(), Duration::from_secs(2)) {
                let _ = s.write_all(KNOCK);
            }
            Ok(Instance::Second)
        }
        Err(std::fs::TryLockError::Error(e)) => Err(e),
    }
}

/// Call `focus` each time another launch knocks.
pub fn listen(listener: TcpListener, focus: impl Fn() + Send + 'static) {
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut buf = [0u8; 8];
            let mut s = stream;
            let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
            if s.read(&mut buf).is_ok_and(|n| &buf[..n] == KNOCK) {
                focus();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_second_claim_knocks_on_the_first() {
        let dir = std::env::temp_dir().join(format!("single-{}", std::process::id()));
        let Instance::First(_lock, listener) = claim(&dir).unwrap() else { panic!("first claim") };
        let (tx, rx) = std::sync::mpsc::channel();
        listen(listener, move || {
            let _ = tx.send(());
        });
        // Another handle to the same file: the lock is per open file, so this acts as a second process.
        assert!(matches!(claim(&dir).unwrap(), Instance::Second));
        rx.recv_timeout(Duration::from_secs(5)).expect("the first instance was asked to focus");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
