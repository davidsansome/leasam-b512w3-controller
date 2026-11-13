//! Lets the HA thread sleep until there's something to do: data from HA on its socket, a
//! request from the UI (signalled through a socket pair), or a timeout.

pub use imp::{Sleeper, Waker, pair};

#[cfg(unix)]
mod imp {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    /// The UI side: pokes the HA thread.
    pub struct Waker(UnixStream);

    /// The HA thread side.
    pub struct Sleeper(UnixStream);

    pub fn pair() -> std::io::Result<(Waker, Sleeper)> {
        let (a, b) = UnixStream::pair()?;
        a.set_nonblocking(true)?;
        b.set_nonblocking(true)?;
        Ok((Waker(a), Sleeper(b)))
    }

    impl Waker {
        pub fn wake(&self) {
            // If the buffer is full, a wake-up is already pending.
            let _ = (&self.0).write(&[1]);
        }
    }

    impl Sleeper {
        /// Waits until `socket` is readable (or writable, if `want_write`), `wake` is
        /// called, or `timeout` passes.
        pub fn wait(&self, socket: &TcpStream, want_write: bool, timeout: Duration) {
            let mut fds = [
                libc::pollfd {
                    fd: socket.as_raw_fd(),
                    events: libc::POLLIN | if want_write { libc::POLLOUT } else { 0 },
                    revents: 0,
                },
                libc::pollfd { fd: self.0.as_raw_fd(), events: libc::POLLIN, revents: 0 },
            ];
            let ms = timeout.as_millis().min(i32::MAX as u128) as libc::c_int;
            // SAFETY: fds is a valid array of two pollfds for the duration of the call.
            unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, ms) };
            let mut buf = [0u8; 64];
            while matches!((&self.0).read(&mut buf), Ok(n) if n > 0) {}
        }
    }
}

#[cfg(not(unix))]
mod imp {
    use std::net::TcpStream;
    use std::time::Duration;

    pub struct Waker;
    pub struct Sleeper;

    pub fn pair() -> std::io::Result<(Waker, Sleeper)> {
        Ok((Waker, Sleeper))
    }

    impl Waker {
        pub fn wake(&self) {}
    }

    impl Sleeper {
        /// No poll() here (the controller only runs on Linux): just nap briefly.
        pub fn wait(&self, _socket: &TcpStream, _want_write: bool, timeout: Duration) {
            std::thread::sleep(timeout.min(Duration::from_millis(100)));
        }
    }
}
