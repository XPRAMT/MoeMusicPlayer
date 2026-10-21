//! The TCP transport the SMB client talks through (ADR 0026).
//!
//! The `smb` crate's own TCP transport never sets `TCP_NODELAY` and sends the four-byte length header and the message
//! as separate writes. Nagle's algorithm holds the second write back until the first is acknowledged, and a server
//! that delays its acknowledgements (about 40 ms) turns every request into a stall: on a real server 2.5 ms away
//! one `stat` took 187 ms instead of about 10 ms. This transport sets `TCP_NODELAY` and sends header and message in a
//! single write. Everything else behaves like the crate's transport (read timeout polling, error mapping).

use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::Duration;

use smb::transport::{
    IoVec, SmbTransport, SmbTransportRead, SmbTransportWrite, TransportError,
};

type Result<T> = std::result::Result<T, TransportError>;

pub struct NoDelayTransport {
    reader: Option<TcpStream>,
    writer: Option<TcpStream>,
    timeout: Duration,
}

impl NoDelayTransport {
    pub const DEFAULT_PORT: u16 = 445;

    pub fn new(timeout: Duration) -> Self {
        Self {
            reader: None,
            writer: None,
            timeout,
        }
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
        stream.set_nodelay(true)?;
        self.reader = Some(stream.try_clone()?);
        self.writer = Some(stream);
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
            }),
            Box::new(Self {
                reader: None,
                writer: self.writer,
                timeout: self.timeout,
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
    fn receive_exact(&mut self, out_buf: &mut [u8]) -> Result<()> {
        let reader = self.reader.as_mut().ok_or(TransportError::NotConnected)?;
        reader.read_exact(out_buf).map_err(Self::map_error)
    }

    fn set_read_timeout(&self, timeout: Duration) -> Result<()> {
        self.reader
            .as_ref()
            .ok_or(TransportError::NotConnected)?
            .set_read_timeout(Some(timeout))
            .map_err(Into::into)
    }
}
