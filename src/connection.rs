use std::io::Cursor;

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
        }
    }

    /// Write a frame to the stream
    async fn write_frame(&mut self, frame: &Frame) -> Result<(), Error> {
        match frame {
            _ => self.write_value(frame).await?,
        };

        self.stream.flush().await.map_err(Error::from)
    }

    async fn write_value(&mut self, frame: &Frame) -> Result<(), Error> {
        todo!()
    }
}
