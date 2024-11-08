use std::{io::Cursor, net::IpAddr};

use bytes::{Buf, BytesMut};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, BufWriter},
    net::TcpStream,
};

use crate::frame::Frame;

// Based on the mini-redis example, found here:
// https://github.com/tokio-rs/mini-redis/blob/tutorial/src/connection.rs

#[derive(Debug)]
pub struct Connection {
    stream: BufWriter<TcpStream>,
    buffer: BytesMut,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Connection reset by peer")]
    ConnectionReset,
    #[error(transparent)]
    FrameError(#[from] crate::frame::Error),
    #[error(transparent)]
    IoError(#[from] std::io::Error),
}

impl Connection {
    /// Create a new `Connection`, based on a `TcpStream`.
    /// Also initializes read and write buffers.
    pub fn new(socket: TcpStream) -> Connection {
        Connection {
            stream: BufWriter::new(socket),
            // Use 4KB buffer, might neet to be adjusted (larger)
            buffer: BytesMut::with_capacity(4 * 1024),
        }
    }

    /// Reads a single `Frame` from the stream.
    ///
    /// Waits until it has received enough data to parse a frame, any
    /// data remaining in the buffer after the frame has been parsed is
    /// kept there for the next call to `read_frame`.
    ///
    /// # Returns
    ///
    /// On success, the received frame. If the connection is closed
    /// without breaking a frame, `None` is returned. Otherwise, an error
    /// is returned.
    pub async fn read_frame(&mut self) -> Result<Option<Frame>, Error> {
        loop {
            // Attempt to parse a frame
            if let Some(frame) = self.parse_frame()? {
                log::info!("Received frame: {:?}", frame);
                return Ok(Some(frame));
            }

            // If we have not enough data, try to read more
            if 0 == self.stream.read_buf(&mut self.buffer).await? {
                // If clean connection shutdown
                if self.buffer.is_empty() {
                    return Ok(None);
                }
                // We were still expecting the rest of a frame
                else {
                    return Err(Error::ConnectionReset);
                }
            }
        }
    }

    /// Tries to parse a frame from the buffer. If the buffer contains enough
    /// data, the frame is returned and the data removed from the buffer. If not
    /// enough data has been buffered yet, `Ok(None)` is returned. If the
    /// buffered data does not represent a valid frame, `Err` is returned.
    fn parse_frame(&mut self) -> Result<Option<Frame>, Error> {
        // Used to track our current location in the buffer
        let mut buf = Cursor::new(&self.buffer[..]);

        // Check if we have enough data to parse a single frame
        // Worth it, as usually much faster than parsing it directly
        match Frame::check(&mut buf) {
            Ok(_) => {
                // Because check advances the cursor
                let len = buf.position() as usize;

                // Start again from the beginning
                buf.set_position(0);

                // Actually read the frame
                let frame = Frame::parse(&mut buf)?;

                // Discard the data from the buffer
                self.buffer.advance(len);

                Ok(Some(frame))
            }
            Err(crate::frame::Error::Incomplete) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Write a frame to the stream
    pub async fn write_frame(&mut self, frame: &Frame) -> Result<(), Error> {
        log::info!("Sending frame: {:?}", frame);
        match frame {
            Frame::DbSync { folder_id } => {
                self.stream.write_u8(b'.').await?;
                self.stream.write_all(&folder_id.to_le_bytes()).await?;
            }
            Frame::Yes => {
                self.stream.write_all(b"+").await?;
            }
            Frame::No => {
                self.stream.write_all(b"-").await?;
            }
            Frame::Done => {
                self.stream.write_all(b"*").await?;
            }
            Frame::InitiatorGlobal {
                global_hash,
                global_last_modified,
                global_peer,
                path,
            } => {
                self.stream.write_u8(b'!').await?;
                self.stream.write_all(&global_hash).await?;
                self.stream
                    .write_all(&global_last_modified.to_le_bytes())
                    .await?;
                self.stream.write_all(global_peer.as_bytes()).await?;
                self.stream.write_all(b"\r\n").await?;
                self.stream.write_all(path.as_bytes()).await?;
                self.stream.write_all(b"\r\n").await?;
            }
            Frame::RequestFile { folder_id, path } => todo!(),
            Frame::File { size, data } => todo!(),
        };

        self.stream.flush().await.map_err(Error::from)
    }

    /// Returns the peer's IP address
    pub fn get_peer_ip(&self) -> Result<IpAddr, Error> {
        Ok(self.stream.get_ref().peer_addr().map_err(Error::from)?.ip())
    }
}

#[cfg(test)]
mod tests {
    use tokio::net::TcpListener;

    use super::*;

    #[tokio::test]
    async fn test_connection_new() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap(); // Don't care about port
        let addr = listener.local_addr().unwrap();

        // Client
        tokio::spawn(async move {
            let _ = TcpStream::connect(addr).await.unwrap();
        });

        // Accept as server
        let (server_socket, _) = listener.accept().await.unwrap();

        let connection = Connection::new(server_socket);

        assert_eq!(connection.buffer.capacity(), 4 * 1024);
    }

    #[tokio::test]
    async fn test_frame_transfer() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        // Client
        tokio::spawn(async move {
            let c = TcpStream::connect(addr).await.unwrap();
            let mut client_connection = Connection::new(c);
            let _ = client_connection.write_frame(&Frame::Yes).await.unwrap();
        });

        // Accept as server
        let (server_socket, _) = listener.accept().await.unwrap();

        let mut connection = Connection::new(server_socket);
        let frame = connection.read_frame().await.unwrap().unwrap();

        assert!(matches!(frame, Frame::Yes));
    }
}
