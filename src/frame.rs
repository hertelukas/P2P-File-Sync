use std::io::Cursor;

use bytes::{Buf, Bytes};

// Based on the mini-redis example, found here:
// https://github.com/tokio-rs/mini-redis/blob/tutorial/src/frame.rs

#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Not enough data available to parse a message
    #[error("Stream ended too early")]
    Incomplete,

}

#[derive(Debug)]
pub enum Frame {
    DbSync {
        folder_id: u64,
    },
    AcceptDbSync,
    InitiatorGlobal {
        global_hash: Bytes,
        global_last_modified: u64,
        global_peer: String
    }
}

impl Frame {
    /// Checks if message can be decoded from `src` and advances
    /// the cursor to the end
    pub fn check(src: &mut Cursor<&[u8]>) -> Result<(), Error> {
        match get_u8(src)? {
            _ => {
                unimplemented!()
            }

        }
    }

    pub fn parse(src: &mut Cursor<&[u8]>) -> Result<Frame, Error> {
        match get_u8(src)? {
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
