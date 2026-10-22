//! The TCP transport the SMB client talks through (ADR 0026).
//!
//! The `smb` crate's own TCP transport never sets `TCP_NODELAY` and sends the four-byte length header and the message
//! as separate writes. Nagle's algorithm holds the second write back until the first is acknowledged, and a server
//! that delays its acknowledgements (about 40 ms) turns every request into a stall: on a real server 2.5 ms away
//! one `stat` took 187 ms instead of about 10 ms. This transport sets `TCP_NODELAY` and sends header and message in a
//! single write. Everything else behaves like the crate's transport (read timeout polling, error mapping).
//!
//! It also tells a slow link from a dead one (ADR 0026, "link lost"): every byte that arrives is noted in the
//! [`LinkHealth`] shared with the caller, so a request may wait as long as data keeps coming, and a link on which
//! nothing at all arrives can be cut ([`LinkHealth::kill`]) to fail everything waiting on it at once. A message is
//! received whole even when the read timeout (the crate polls every 100 ms) falls in the middle of it: what was read
//! is kept and the next call goes on from there, so the framing never slips.

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use smb::transport::{
    IoVec, SmbTransport, SmbTransportRead, SmbTransportWrite, TransportError,
};

type Result<T> = std::result::Result<T, TransportError>;

/// What a connection's receiving side has seen: when the last byte arrived and how many came. Shared between the
/// transport (which notes every byte) and whoever waits for an answer on the connection.
pub struct LinkHealth {
    born: Instant,
    /// Milliseconds since `born` at which the last byte arrived (0: none yet).
    last_rx_ms: AtomicU64,
    rx_bytes: AtomicU64,
    killed: AtomicBool,
    /// A handle on the socket, to shut it down from another thread.
    socket: Mutex<Option<TcpStream>>,
}

impl LinkHealth {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            born: Instant::now(),
            last_rx_ms: AtomicU64::new(0),
            rx_bytes: AtomicU64::new(0),
            killed: AtomicBool::new(false),
            socket: Mutex::new(None),
        })
    }

    fn now_ms(&self) -> u64 {
        self.born.elapsed().as_millis() as u64 + 1
    }

    /// `count` bytes just arrived.
    pub fn note_received(&self, count: usize) {
        self.rx_bytes.fetch_add(count as u64, Ordering::Relaxed);
        self.last_rx_ms.store(self.now_ms(), Ordering::Relaxed);
    }

    /// Bytes received so far.
    pub fn received(&self) -> u64 {
        self.rx_bytes.load(Ordering::Relaxed)
    }

    /// When the last byte arrived, or `None` before the first.
    pub fn last_received(&self) -> Option<Instant> {
        match self.last_rx_ms.load(Ordering::Relaxed) {
            0 => None,
            ms => Some(self.born + Duration::from_millis(ms - 1)),
        }
    }

    fn attach(&self, socket: TcpStream) {
        *self.socket.lock().unwrap_or_else(PoisonError::into_inner) = Some(socket);
    }

    /// Cut the connection: the socket is shut down, so the receive loop ends and every request waiting on it fails.
    pub fn kill(&self) {
        self.killed.store(true, Ordering::SeqCst);
        if let Some(socket) = self.socket.lock().unwrap_or_else(PoisonError::into_inner).as_ref() {
            let _ = socket.shutdown(std::net::Shutdown::Both);
        }
    }

    pub fn is_killed(&self) -> bool {
        self.killed.load(Ordering::SeqCst)
    }
}

/// A message being received: its 4-byte transport header, then its body, each filled across as many calls as it takes.
#[derive(Default)]
struct Partial {
    header: [u8; 4],
    header_filled: usize,
    body: Option<Vec<u8>>,
    body_filled: usize,
}

pub struct NoDelayTransport {
    reader: Option<TcpStream>,
    writer: Option<TcpStream>,
    timeout: Duration,
    health: Arc<LinkHealth>,
    partial: Partial,
}

impl NoDelayTransport {
    pub const DEFAULT_PORT: u16 = 445;

    pub fn new(timeout: Duration) -> Self {
        Self::with_health(timeout, LinkHealth::new())
    }

    pub fn with_health(timeout: Duration, health: Arc<LinkHealth>) -> Self {
        Self {
            reader: None,
            writer: None,
            timeout,
            health,
            partial: Partial::default(),
        }
    }

    /// Over a socket that is already connected (tests).
    #[cfg(test)]
    fn over(stream: TcpStream, health: Arc<LinkHealth>) -> Self {
        let mut transport = Self::with_health(Duration::ZERO, health);
        transport.adopt(stream).expect("a connected socket");
        transport
    }

    fn adopt(&mut self, stream: TcpStream) -> io::Result<()> {
        stream.set_nodelay(true)?;
        self.health.attach(stream.try_clone()?);
        self.reader = Some(stream.try_clone()?);
        self.writer = Some(stream);
        Ok(())
    }

    /// Read into `out[*filled..]` until it is full. A read timeout returns `WouldBlock` (what the crate's receive loop
    /// polls on) and keeps `filled`, so the next call continues where this one stopped.
    fn fill(
        reader: &mut TcpStream,
        health: &LinkHealth,
        out: &mut [u8],
        filled: &mut usize,
    ) -> Result<()> {
        while *filled < out.len() {
            match reader.read(&mut out[*filled..]) {
                Ok(0) => return Err(TransportError::NotConnected),
                Ok(count) => {
                    *filled += count;
                    health.note_received(count);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error)
                    if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) =>
                {
                    if health.is_killed() {
                        return Err(TransportError::NotConnected);
                    }
                    return Err(io::Error::from(io::ErrorKind::WouldBlock).into());
                }
                Err(error) => return Err(Self::map_error(error)),
            }
        }
        Ok(())
    }

    fn map_error(error: io::Error) -> TransportError {
        match error.kind() {
            io::ErrorKind::ConnectionAborted | io::ErrorKind::UnexpectedEof => {
                TransportError::NotConnected
            }
            _ => error.into(),
        }
    }
}

impl SmbTransport for NoDelayTransport {
    fn connect(&mut self, _server_name: &str, address: SocketAddr) -> Result<()> {
        let stream = if self.timeout == Duration::ZERO {
            TcpStream::connect(address)?
        } else {
            TcpStream::connect_timeout(&address, self.timeout).map_err(|error| {
                if error.kind() == io::ErrorKind::TimedOut {
                    TransportError::Timeout(self.timeout)
                } else {
                    error.into()
                }
            })?
        };
        self.adopt(stream)?;
        Ok(())
    }

    fn default_port(&self) -> u16 {
        Self::DEFAULT_PORT
    }

    fn split(self: Box<Self>) -> Result<(Box<dyn SmbTransportRead>, Box<dyn SmbTransportWrite>)> {
        Ok((
            Box::new(Self {
                reader: self.reader,
                writer: None,
                timeout: self.timeout,
                health: Arc::clone(&self.health),
                partial: self.partial,
            }),
            Box::new(Self {
                reader: None,
                writer: self.writer,
                timeout: self.timeout,
                health: self.health,
                partial: Partial::default(),
            }),
        ))
    }

    fn remote_address(&self) -> Result<SocketAddr> {
        self.reader
            .as_ref()
            .or(self.writer.as_ref())
            .ok_or(TransportError::NotConnected)?
            .peer_addr()
            .map_err(Into::into)
    }
}

impl SmbTransportWrite for NoDelayTransport {
    fn send_raw(&mut self, buf: &[u8]) -> Result<()> {
        let writer = self.writer.as_mut().ok_or(TransportError::NotConnected)?;
        writer.write_all(buf).map_err(Self::map_error)
    }

    /// The transport header and the whole message in one write (the default sends each piece on its own).
    fn send(&mut self, data: &IoVec) -> Result<()> {
        let total = data.total_size();
        let mut frame = Vec::with_capacity(4 + total);
        // Direct TCP framing: a zero byte and a 24-bit big-endian length.
        frame.push(0);
        frame.extend_from_slice(&(total as u32).to_be_bytes()[1..]);
        for buf in data.iter() {
            frame.extend_from_slice(buf);
        }
        self.send_raw(&frame)
    }
}

impl SmbTransportRead for NoDelayTransport {
    /// For callers of the trait method other than [`Self::receive`]: whole or an error, so a read timeout after some
    /// bytes waits on (they cannot be handed back without breaking the framing).
    fn receive_exact(&mut self, out_buf: &mut [u8]) -> Result<()> {
        let reader = self.reader.as_mut().ok_or(TransportError::NotConnected)?;
        let mut filled = 0;
        loop {
            match Self::fill(reader, &self.health, out_buf, &mut filled) {
                Err(TransportError::IoError(error))
                    if error.kind() == io::ErrorKind::WouldBlock && filled > 0 => {}
                other => return other,
            }
        }
    }

    /// One whole message (without its transport header). A read timeout in the middle returns `WouldBlock` and keeps
    /// what arrived, so the next call finishes the same message.
    fn receive(&mut self) -> Result<Vec<u8>> {
        let reader = self.reader.as_mut().ok_or(TransportError::NotConnected)?;
        let partial = &mut self.partial;
        if partial.body.is_none() {
            Self::fill(reader, &self.health, &mut partial.header, &mut partial.header_filled)?;
            // Direct TCP framing: a zero byte and a 24-bit big-endian length.
            let header = partial.header;
            let length = u32::from_be_bytes([0, header[1], header[2], header[3]]) as usize;
            partial.body = Some(vec![0; length]);
            partial.body_filled = 0;
        }
        let body = partial.body.as_mut().expect("set above");
        Self::fill(reader, &self.health, body, &mut partial.body_filled)?;
        let message = partial.body.take().expect("set above");
        *partial = Partial::default();
        Ok(message)
    }

    fn set_read_timeout(&self, timeout: Duration) -> Result<()> {
        self.reader
            .as_ref()
            .ok_or(TransportError::NotConnected)?
            .set_read_timeout(Some(timeout))
            .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smb::{Watch, Watchdog};
    use std::net::TcpListener;

    /// A connected pair: our transport on one side (100 ms read timeout, as the crate's receive loop sets it), the
    /// "server" socket on the other.
    fn pair() -> (NoDelayTransport, TcpStream, Arc<LinkHealth>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        let health = LinkHealth::new();
        let transport = NoDelayTransport::over(client, Arc::clone(&health));
        SmbTransportRead::set_read_timeout(&transport, Duration::from_millis(100)).unwrap();
        (transport, server, health)
    }

    fn frame(body: &[u8]) -> Vec<u8> {
        let mut frame = vec![0];
        frame.extend_from_slice(&(body.len() as u32).to_be_bytes()[1..]);
        frame.extend_from_slice(body);
        frame
    }

    fn would_block(result: &Result<Vec<u8>>) -> bool {
        matches!(result, Err(TransportError::IoError(error)) if error.kind() == io::ErrorKind::WouldBlock)
    }

    /// Receive like the crate's loop does: poll, skipping `WouldBlock`.
    fn receive_polling(transport: &mut NoDelayTransport) -> (Result<Vec<u8>>, usize) {
        let mut polls = 0;
        loop {
            let result = SmbTransportRead::receive(transport);
            if would_block(&result) {
                polls += 1;
                continue;
            }
            return (result, polls);
        }
    }

    #[test]
    fn a_message_cut_into_pieces_with_timeouts_in_between_arrives_whole_and_the_next_one_too() {
        let (mut transport, mut server, health) = pair();
        let first = frame(b"hello, slow world");
        let second = frame(&[7_u8; 3000]);
        let writer = std::thread::spawn(move || {
            // Split inside the header, inside the body, and across the two messages; each pause is longer than the
            // read timeout.
            let mut bytes = first.clone();
            bytes.extend_from_slice(&second);
            for piece in [&bytes[..2], &bytes[2..9], &bytes[9..first.len() + 1], &bytes[first.len() + 1..]] {
                server.write_all(piece).unwrap();
                server.flush().unwrap();
                std::thread::sleep(Duration::from_millis(250));
            }
            server
        });
        let (message, polls) = receive_polling(&mut transport);
        assert_eq!(message.unwrap(), b"hello, slow world");
        assert!(polls >= 2, "the timeouts fell inside the message {polls} times");
        let (message, _) = receive_polling(&mut transport);
        assert_eq!(message.unwrap(), vec![7_u8; 3000]);
        assert_eq!(health.received(), (4 + 17 + 4 + 3000) as u64);
        drop(writer.join().unwrap());
    }

    /// Drive a watchdog for one "request" until `done` or a verdict of dead; returns the verdict and when.
    fn watch_until(
        health: &LinkHealth,
        done: &std::sync::atomic::AtomicBool,
        limit: Duration,
    ) -> (Option<Duration>, usize) {
        let started = Instant::now();
        let mut watchdog = Watchdog::new(started, Duration::from_secs(1), Duration::from_secs(1));
        let mut echoes = 0;
        while !done.load(Ordering::SeqCst) && started.elapsed() < limit {
            match watchdog.check(Instant::now(), health.last_received()) {
                Watch::Wait => {}
                Watch::SendEcho => echoes += 1,
                Watch::Dead => return (Some(started.elapsed()), echoes),
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        (None, echoes)
    }

    #[test]
    fn one_byte_every_half_second_is_slow_not_dead() {
        let (mut transport, mut server, health) = pair();
        let body = b"slowly".to_vec();
        let bytes = frame(&body);
        let writer = std::thread::spawn(move || {
            for byte in bytes {
                server.write_all(&[byte]).unwrap();
                std::thread::sleep(Duration::from_millis(500));
            }
            server
        });
        let done = Arc::new(AtomicBool::new(false));
        let receiver_done = Arc::clone(&done);
        let receiver = std::thread::spawn(move || {
            let (message, _) = receive_polling(&mut transport);
            receiver_done.store(true, Ordering::SeqCst);
            message.unwrap()
        });
        // 10 bytes at 0.5 s: about 5 s for the message, far past the 2 s a dead link takes.
        let (dead, echoes) = watch_until(&health, &done, Duration::from_secs(20));
        assert_eq!(dead, None, "a link that sends a byte every 0.5 s was called dead");
        assert_eq!(echoes, 0);
        assert_eq!(receiver.join().unwrap(), body);
        drop(writer.join().unwrap());
    }

    #[test]
    fn a_link_on_which_nothing_arrives_is_dead_after_the_echo_wait_and_cutting_it_ends_the_receiver() {
        let (mut transport, mut server, health) = pair();
        // Half a message, then silence (the server neither sends nor closes).
        server.write_all(&frame(b"never finished")[..8]).unwrap();
        let done = AtomicBool::new(false);
        let receiver = std::thread::spawn(move || receive_polling(&mut transport).0);
        std::thread::sleep(Duration::from_millis(100));
        let (dead, echoes) = watch_until(&health, &done, Duration::from_secs(10));
        let dead = dead.expect("the link was not found dead");
        assert!(
            dead >= Duration::from_millis(1900) && dead < Duration::from_millis(3000),
            "found dead after {dead:?}"
        );
        assert_eq!(echoes, 1);
        let cut = Instant::now();
        health.kill();
        let result = receiver.join().unwrap();
        assert!(matches!(result, Err(TransportError::NotConnected)), "{result:?}");
        assert!(cut.elapsed() < Duration::from_secs(1));
        drop(server);
    }
}
