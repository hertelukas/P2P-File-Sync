use std::io::Cursor;

use bytes::{Buf, Bytes};
use std::string::FromUtf8Error;
// Based on the mini-redis example, found here:
// https://github.com/tokio-rs/mini-redis/blob/tutorial/src/frame.rs

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Not enough data available to parse a message
    #[error("Stream ended too early")]
    Incomplete,
    #[error(transparent)]
    ParseError(#[from] FromUtf8Error),
}

#[derive(Debug)]
pub enum Frame {
    // '.'
    DbSync {
        folder_id: u32,
    },
    // '+'
    Yes,
    // '-'
    No,
    // *
    Done,
    // '!'
    InitiatorGlobal {
        global_hash: Bytes,
        global_last_modified: i64,
        global_peer: String,
        path: String,
    },
    // ?
    RequestFile {
        folder_id: u32,
        path: String,
    },
    // =
    File {
        size: i64,
        data: Bytes,
    },
}

impl Frame {
    /// Checks if message can be decoded from `src` and advances
    /// the cursor to the end
    pub fn check(src: &mut Cursor<&[u8]>) -> Result<(), Error> {
        match get_u8(src)? {
            // DbSync
            b'.' => {
                // Skip 4 bytes folder_id
                skip(src, 4)
            }
            // Yes
            b'+' => Ok(()),
            // No
            b'-' => Ok(()),
            // Done
            b'*' => Ok(()),
            // InitiatorGlobal
            b'!' => {
                // Sha256 (32) + last_modified (8) + string + string
                skip(src, 40)?;
                get_line(src)?;
                get_line(src)?;
                Ok(())
            }
            b'?' => {
                // Skip 4 bytes folder_id
                skip(src, 4)?;
                get_line(src)?;
                Ok(())
            }
            b'=' => {
                let size: i64 = src.get_i64_le();
                skip(src, size.try_into().unwrap())
            }
            _ => {
                unimplemented!()
            }
        }
    }

    pub fn parse(src: &mut Cursor<&[u8]>) -> Result<Frame, Error> {
        match get_u8(src)? {
            // DbSync
            b'.' => {
                let folder_id: u32 = src.get_u32_le();
                Ok(Frame::DbSync { folder_id })
            }
            b'+' => Ok(Frame::Yes),
            b'-' => Ok(Frame::No),
            b'*' => Ok(Frame::Done),
            b'!' => {
                let global_hash = Bytes::copy_from_slice(&src.chunk()[..32]);
                skip(src, 32)?;
                let global_last_modified = src.get_i64_le();
                let line = get_line(src)?.to_vec();
                let global_peer = String::from_utf8(line)?;
                let line = get_line(src)?.to_vec();
                let path = String::from_utf8(line)?;
                Ok(Frame::InitiatorGlobal {
                    global_hash,
                    global_last_modified,
                    global_peer,
                    path,
                })
            }
            b'?' => {
                let folder_id: u32 = src.get_u32_le();
                let line = get_line(src)?.to_vec();
                let path = String::from_utf8(line)?;
                Ok(Frame::RequestFile { folder_id, path })
            }
            b'=' => {
                let size: i64 = src.get_i64_le();
                let data = Bytes::copy_from_slice(&src.chunk()[..size.try_into().unwrap()]);
                Ok(Frame::File { size, data })
            }
            _ => {
                unimplemented!()
            }
        }
    }
}

fn get_u8(src: &mut Cursor<&[u8]>) -> Result<u8, Error> {
    if !src.has_remaining() {
        return Err(Error::Incomplete);
    }
    Ok(src.get_u8())
}

fn skip(src: &mut Cursor<&[u8]>, n: usize) -> Result<(), Error> {
    if src.remaining() < n {
        return Err(Error::Incomplete);
    }

    src.advance(n);
    Ok(())
}

/// Find a line
fn get_line<'a>(src: &mut Cursor<&'a [u8]>) -> Result<&'a [u8], Error> {
    // Scan the bytes directly
    let start = src.position() as usize;
    // Scan to the second to last byte
    let end = src.get_ref().len() - 1;

    for i in start..end {
        if src.get_ref()[i] == b'\r' && src.get_ref()[i + 1] == b'\n' {
            // We found a line, update the position to be *after* the \n
            src.set_position((i + 2) as u64);

            // Return the line
            return Ok(&src.get_ref()[start..i]);
        }
    }

    Err(Error::Incomplete)
}

#[cfg(test)]
mod tests {
    use bytes::{BufMut, BytesMut};

    use super::*;

    #[test]
    fn test_check_db_sync() {
        let mut buf = BytesMut::with_capacity(1024);
        buf.put_u8(b'.');
        buf.put(&b"\xFF\xFF\xFF\xFF"[..]);

        let mut buf = Cursor::new(&buf[..]);

        Frame::check(&mut buf).unwrap();
    }

    #[test]
    fn test_parse_db_sync() {
        let mut buf = BytesMut::with_capacity(1024);
        buf.put_u8(b'.');
        buf.put(&b"\xFF\xFF\xFF\xFF"[..]);

        let mut buf = Cursor::new(&buf[..]);

        let frame = Frame::parse(&mut buf).unwrap();

        match frame {
            Frame::DbSync { folder_id } => assert_eq!(folder_id, 0xFFFFFFFF),
            _ => assert!(false),
        }
    }

    #[test]
    #[should_panic]
    fn test_short_fails() {
        let mut buf = BytesMut::with_capacity(1024);
        buf.put_u8(b'.');
        buf.put(&b"\xFF\xFF\xFF"[..]);

        let mut buf = Cursor::new(&buf[..]);
        Frame::check(&mut buf).unwrap();
    }

    #[test]
    fn test_yes() {
        let mut buf = BytesMut::with_capacity(1024);
        buf.put_u8(b'+');

        let mut buf = Cursor::new(&buf[..]);

        Frame::check(&mut buf).unwrap();
        buf.set_position(0);
        assert!(matches!(Frame::parse(&mut buf).unwrap(), Frame::Yes));
    }

    #[test]
    fn test_no() {
        let mut buf = BytesMut::with_capacity(1024);
        buf.put_u8(b'-');

        let mut buf = Cursor::new(&buf[..]);

        Frame::check(&mut buf).unwrap();
        buf.set_position(0);
        assert!(matches!(Frame::parse(&mut buf).unwrap(), Frame::No));
    }

    #[test]
    fn test_done() {
        let mut buf = BytesMut::with_capacity(1024);
        buf.put_u8(b'*');

        let mut buf = Cursor::new(&buf[..]);

        Frame::check(&mut buf).unwrap();
        buf.set_position(0);
        assert!(matches!(Frame::parse(&mut buf).unwrap(), Frame::Done));
    }

    #[test]
    fn test_initiator_global() {
        let mut buf = BytesMut::with_capacity(1024);
        buf.put_u8(b'!');
        // 32 Bytes Sha256
        buf.extend(std::iter::repeat(b'\x42').take(32));
        // 8 bytes global last modified
        buf.extend(std::iter::repeat(b'\x12').take(8));
        // And the IP of the global peer as String
        buf.put("127.0.0.1".as_bytes());
        // and terminate
        buf.put(&b"\r\n"[..]);
        // And the path as String
        buf.put("folder/file".as_bytes());
        // and terminate
        buf.put(&b"\r\n"[..]);

        let mut buf = Cursor::new(&buf[..]);

        Frame::check(&mut buf).unwrap();
        buf.set_position(0);
        match Frame::parse(&mut buf).unwrap() {
            Frame::InitiatorGlobal {
                global_hash,
                global_last_modified,
                global_peer,
                path,
            } => {
                let bytes = Bytes::from(vec![0x42; 32]);
                assert_eq!(global_hash, bytes);
                assert_eq!(global_last_modified, 0x1212121212121212);
                assert_eq!(global_peer, "127.0.0.1");
                assert_eq!(path, "folder/file");
            }
            _ => assert!(false),
        }
    }

    #[test]
    fn test_request_file() {
        let mut buf = BytesMut::with_capacity(1024);
        buf.put_u8(b'?');
        // Folder_id
        buf.put(&b"\xFF\xFF\xFF\xFF"[..]);
        // Path
        buf.put("folder/file".as_bytes());
        // and terminate
        buf.put(&b"\r\n"[..]);

        let mut buf = Cursor::new(&buf[..]);

        Frame::check(&mut buf).unwrap();
        buf.set_position(0);
        match Frame::parse(&mut buf).unwrap() {
            Frame::RequestFile { folder_id, path } => {
                assert_eq!(folder_id, 0xFFFFFFFF);
                assert_eq!(path, "folder/file");
            }
            _ => assert!(false),
        }
    }

    #[test]
    fn test_file() {
        let mut buf = BytesMut::with_capacity(1024);
        buf.put_u8(b'=');
        // 8 bytes size of 16
        buf.put(&b"\x10\x00\x00\x00\x00\x00\x00\x00"[..]);
        // 16 byte file
        buf.extend(std::iter::repeat(b'\x42').take(16));

        let mut buf = Cursor::new(&buf[..]);

        Frame::check(&mut buf).unwrap();
        buf.set_position(0);
        match Frame::parse(&mut buf).unwrap() {
            Frame::File { size, data } => {
                let bytes = Bytes::from(vec![0x42; 16]);
                assert_eq!(size, 16);
                assert_eq!(data, bytes);
            }
            _ => assert!(false),
        }
    }

    #[test]
    #[should_panic]
    fn test_short_file() {
        let mut buf = BytesMut::with_capacity(1024);
        buf.put_u8(b'=');
        // 8 bytes size of 32
        buf.put(&b"\x20\x00\x00\x00\x00\x00\x00\x00"[..]);
        // 16 byte file
        buf.extend(std::iter::repeat(b'\x42').take(16));

        let mut buf = Cursor::new(&buf[..]);

        Frame::check(&mut buf).unwrap();
    }

}
