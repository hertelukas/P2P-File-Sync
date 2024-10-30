use std::io::{Cursor, Read};

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
    // '!'
    InitiatorGlobal {
        global_hash: Bytes,
        global_last_modified: u64,
        global_peer: String,
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
            // InitiatorGlobal
            b'!' => {
                // Sha256 (32) + last_modified (8) + string
                skip(src, 40)?;
                get_line(src)?;
                Ok(())
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
                let folder_id: u32 = src.get_u32_ne();
                Ok(Frame::DbSync { folder_id })
            }
            b'+' => Ok(Frame::Yes),
            b'-' => Ok(Frame::No),
            b'!' => {
                let global_hash = Bytes::copy_from_slice(&src.chunk()[..32]);
                skip(src, 32)?;
                let global_last_modified = src.get_u64_ne();
                let line = get_line(src)?.to_vec();
                let global_peer = String::from_utf8(line)?;
                Ok(Frame::InitiatorGlobal {
                    global_hash,
                    global_last_modified,
                    global_peer,
                })
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
