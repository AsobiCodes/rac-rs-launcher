//! A reader that cannot hang the caller. A network body read can block for minutes on a dead connection, and while it blocks the
//! caller cannot even look at a Cancel button. `StallGuard` reads the real body on its own thread, in 64 KiB chunks, and hands them over:
//! `read` waits at most `POLL` for the next chunk and otherwise answers `Interrupted` (the caller checks Cancel and reads again), and after
//! `idle` without a single byte it answers `TimedOut`. The end of the body is `Ok(0)` as usual. A thread stuck inside a dead connection is
//! left behind and ends when the connection does; nothing waits for it.
use std::io::{self, Read};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

const CHUNK: usize = 64 * 1024;
/// How long one `read` waits for data before it hands control back with `Interrupted`.
pub const POLL: Duration = Duration::from_millis(250);

pub struct StallGuard {
    rx: Receiver<io::Result<Vec<u8>>>,
    chunk: Vec<u8>,
    pos: usize,
    idle: Duration,
    last_data: Instant,
    ended: bool,
}

impl StallGuard {
    /// `idle` = how long no data at all may arrive before `read` fails with `TimedOut`.
    pub fn new(mut inner: impl Read + Send + 'static, idle: Duration) -> StallGuard {
        let (tx, rx) = sync_channel(4);
        std::thread::spawn(move || loop {
            let mut buf = vec![0u8; CHUNK];
            match inner.read(&mut buf) {
                // the end: dropping `tx` tells the reader
                Ok(0) => break,
                Ok(n) => {
                    buf.truncate(n);
                    if tx.send(Ok(buf)).is_err() {
                        break;
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => {
                    let _ = tx.send(Err(e));
                    break;
                }
            }
        });
        StallGuard { rx, chunk: Vec::new(), pos: 0, idle, last_data: Instant::now(), ended: false }
    }

    fn hand_out(&mut self, out: &mut [u8]) -> usize {
        let n = out.len().min(self.chunk.len() - self.pos);
        out[..n].copy_from_slice(&self.chunk[self.pos..self.pos + n]);
        self.pos += n;
        n
    }
}

impl Read for StallGuard {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        if out.is_empty() {
            return Ok(0);
        }
        if self.pos < self.chunk.len() {
            return Ok(self.hand_out(out));
        }
        if self.ended {
            return Ok(0);
        }
        match self.rx.recv_timeout(POLL) {
            Ok(Ok(chunk)) => {
                self.chunk = chunk;
                self.pos = 0;
                self.last_data = Instant::now();
                Ok(self.hand_out(out))
            }
            Ok(Err(e)) => {
                self.ended = true;
                Err(e)
            }
            Err(RecvTimeoutError::Timeout) if self.last_data.elapsed() >= self.idle => Err(io::Error::new(io::ErrorKind::TimedOut, "no data for too long")),
            Err(RecvTimeoutError::Timeout) => Err(io::ErrorKind::Interrupted.into()),
            Err(RecvTimeoutError::Disconnected) => {
                self.ended = true;
                Ok(0)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Delivers `data`, then blocks until `release` is set (a dead connection when it never is).
    struct Slow {
        data: Vec<u8>,
        at: usize,
        release: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl Read for Slow {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            if self.at < self.data.len() {
                let n = out.len().min(self.data.len() - self.at).min(1000);
                out[..n].copy_from_slice(&self.data[self.at..self.at + n]);
                self.at += n;
                return Ok(n);
            }
            while !self.release.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(0)
        }
    }

    fn slow(data: Vec<u8>, released: bool) -> (Slow, std::sync::Arc<std::sync::atomic::AtomicBool>) {
        let release = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(released));
        (Slow { data, at: 0, release: release.clone() }, release)
    }

    #[test]
    fn a_normal_body_comes_through_whole() {
        let data: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let (r, _) = slow(data.clone(), true);
        let mut g = StallGuard::new(r, Duration::from_secs(5));
        let mut got = Vec::new();
        loop {
            let mut b = [0u8; 4096];
            match g.read(&mut b) {
                Ok(0) => break,
                Ok(n) => got.extend_from_slice(&b[..n]),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => panic!("{e}"),
            }
        }
        assert_eq!(got, data);
    }

    #[test]
    fn a_connection_that_goes_silent_answers_quickly_and_then_times_out() {
        let (r, release) = slow(vec![7u8; 10], false);
        let mut g = StallGuard::new(r, Duration::from_secs(1));
        let mut b = [0u8; 64];
        assert_eq!(g.read(&mut b).unwrap(), 10);
        let start = Instant::now();
        // every silent read comes back within about a quarter of a second so the caller can look at Cancel
        let first = g.read(&mut b).unwrap_err();
        assert_eq!(first.kind(), io::ErrorKind::Interrupted);
        assert!(start.elapsed() < Duration::from_millis(900), "{:?}", start.elapsed());
        let err = loop {
            match g.read(&mut b) {
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                other => break other.unwrap_err(),
            }
        };
        assert_eq!(err.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(4), "{:?}", start.elapsed());
        release.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    #[test]
    fn an_error_of_the_body_reaches_the_reader_once_and_then_it_is_over() {
        struct Boom(bool);
        impl Read for Boom {
            fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
                if self.0 {
                    return Err(io::Error::new(io::ErrorKind::ConnectionReset, "reset"));
                }
                self.0 = true;
                out[0] = 1;
                Ok(1)
            }
        }
        let mut g = StallGuard::new(Boom(false), Duration::from_secs(5));
        let mut b = [0u8; 8];
        assert_eq!(g.read(&mut b).unwrap(), 1);
        assert_eq!(g.read(&mut b).unwrap_err().kind(), io::ErrorKind::ConnectionReset);
        assert_eq!(g.read(&mut b).unwrap(), 0);
    }
}
