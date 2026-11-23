//! Jensen frame codec.
//!
//! ```text
//! 0     2        4            8            12
//! magic command  sequence     length       body
//! ```
//!
//! Magic is `0x12 0x34`. Multi-byte fields are big-endian.
//! On a reply, the top byte of `length` is padding. The low 24 bits are the
//! body length. This crate sends padding `0`.

use crate::error::{Error, Result};
use crate::ids::MAX_BODY_LEN;

#[derive(Debug)]
pub(crate) struct Frame {
    pub(crate) command: u16,
    pub(crate) sequence: u32,
    pub(crate) body: Vec<u8>,
}

/// Encode one command frame.
pub(crate) fn encode(command: u16, sequence: u32, body: &[u8]) -> Result<Vec<u8>> {
    if body.len() > MAX_BODY_LEN {
        return Err(Error::BodyTooLarge {
            command,
            length: body.len(),
            max: MAX_BODY_LEN,
        });
    }
    let mut out = Vec::with_capacity(12 + body.len());
    out.extend_from_slice(&[0x12, 0x34]);
    out.extend_from_slice(&command.to_be_bytes());
    out.extend_from_slice(&sequence.to_be_bytes());
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(body);
    Ok(out)
}

/// Take one frame from the front of `input`.
///
/// `Ok(None)` means the buffer does not hold a full frame yet.
pub(crate) fn decode(input: &[u8]) -> Result<Option<(Frame, usize)>> {
    if input.len() < 2 {
        return Ok(None);
    }
    if input[0] != 0x12 || input[1] != 0x34 {
        return Err(Error::BadMagic {
            magic: u16::from_be_bytes([input[0], input[1]]),
        });
    }
    if input.len() < 12 {
        return Ok(None);
    }
    let command = u16::from_be_bytes([input[2], input[3]]);
    let sequence = u32::from_be_bytes([input[4], input[5], input[6], input[7]]);
    let raw = u32::from_be_bytes([input[8], input[9], input[10], input[11]]);
    let padding = (raw >> 24) as usize;
    let body_len = (raw & 0x00ff_ffff) as usize;
    if body_len > MAX_BODY_LEN {
        return Err(Error::BodyTooLarge {
            command,
            length: body_len,
            max: MAX_BODY_LEN,
        });
    }
    let total = 12 + body_len + padding;
    if input.len() < total {
        return Ok(None);
    }
    let body = input[12..12 + body_len].to_vec();
    Ok(Some((
        Frame {
            command,
            sequence,
            body,
        },
        total,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_uses_a_zero_padding_byte() {
        let bytes = encode(4, 7, b"abc").unwrap();
        assert_eq!(bytes.len(), 15);
        assert_eq!(&bytes[..2], &[0x12, 0x34]);
        assert_eq!(&bytes[8..12], &3u32.to_be_bytes());
        let (frame, used) = decode(&bytes).unwrap().unwrap();
        assert_eq!(used, 15);
        assert_eq!(frame.command, 4);
        assert_eq!(frame.sequence, 7);
        assert_eq!(frame.body, b"abc");
    }

    #[test]
    fn reply_padding_is_not_part_of_the_body() {
        let mut bytes = encode(1, 1, &[0xaa]).unwrap();
        bytes[8] = 1;
        bytes.push(0xff);
        let (frame, used) = decode(&bytes).unwrap().unwrap();
        assert_eq!(used, 14);
        assert_eq!(frame.body, &[0xaa]);
    }

    #[test]
    fn a_short_buffer_waits_for_the_rest() {
        let bytes = encode(1, 0, &[1, 2, 3, 4]).unwrap();
        assert!(decode(&bytes[..11]).unwrap().is_none());
        assert!(decode(&bytes[..15]).unwrap().is_none());
        assert!(decode(&bytes).unwrap().is_some());
    }

    #[test]
    fn a_bad_magic_is_an_error() {
        let err = decode(&[0x00, 0x01]).unwrap_err();
        assert!(matches!(err, Error::BadMagic { magic: 0x0001 }));
    }

    #[test]
    fn a_declared_body_over_the_cap_fails_before_the_bytes_arrive() {
        let mut header = [0u8; 12];
        header[0] = 0x12;
        header[1] = 0x34;
        header[8..12].copy_from_slice(&(MAX_BODY_LEN as u32 + 1).to_be_bytes());
        let err = decode(&header).unwrap_err();
        assert!(matches!(
            err,
            Error::BodyTooLarge { length, .. } if length == MAX_BODY_LEN + 1
        ));
    }
}
