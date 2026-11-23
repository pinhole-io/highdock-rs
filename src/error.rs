//! Failures a caller can handle without a raw USB dump.

use std::fmt;

/// Crate result.
pub type Result<T> = std::result::Result<T, Error>;

/// A device or protocol failure.
///
/// A poisoned session cannot send another command. Drop [`crate::Device`] and
/// reconnect the recorder.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// No matched recorder is connected.
    #[error("no HiDock recorder found")]
    NotFound,

    /// The USB device closed the transfer.
    #[error("the recorder closed the usb transfer")]
    Disconnected,

    /// A read or write exceeded its wait.
    #[error("timed out waiting for command {command}")]
    Timeout {
        /// Jensen command id that was in flight.
        command: u16,
    },

    /// The USB stack returned before a protocol timeout.
    #[error("timed out waiting for usb data")]
    IoTimeout,

    /// The session saw a fault and must not send another command.
    #[error("the session is unusable after a protocol fault; reconnect the recorder")]
    Poisoned,

    /// The first two bytes were not `0x12 0x34`.
    #[error("response magic was {magic:#06x}, expected 0x1234")]
    BadMagic {
        /// The two header bytes as a big-endian value.
        magic: u16,
    },

    /// The header declared a body this crate refuses to buffer.
    #[error("command {command} body length {length} exceeds {max}")]
    BodyTooLarge {
        /// Jensen command id from the header. Zero when the frame was not decoded.
        command: u16,
        /// Declared body length.
        length: usize,
        /// The crate limit.
        max: usize,
    },

    /// The carry buffer grew past one maximum frame.
    #[error("a frame exceeded {max} bytes before it completed")]
    FrameTooLarge {
        /// Carry limit in bytes.
        max: usize,
    },

    /// The reply did not match the command that is in flight.
    #[error(
        "reply command {got} sequence {got_sequence} did not match request {expected} sequence {expected_sequence}"
    )]
    UnexpectedReply {
        /// Command id in the reply.
        got: u16,
        /// Sequence in the reply.
        got_sequence: u32,
        /// Command id that was sent.
        expected: u16,
        /// Sequence that was sent.
        expected_sequence: u32,
    },

    /// A payload failed a length or range check.
    #[error("command {command} payload is malformed")]
    Malformed {
        /// Jensen command id.
        command: u16,
    },

    /// The file name cannot be sent to this model.
    #[error("file name is not valid for this recorder")]
    InvalidFileName,

    /// The recording length is outside the transfer limit.
    #[error("recording length {length} is outside the supported range")]
    InvalidLength {
        /// The rejected length in bytes.
        length: u64,
    },

    /// The clock fields are not a real civil time.
    #[error("clock fields are not a valid time")]
    InvalidTime,

    /// The connected firmware does not implement this command.
    #[error("command {command} is not available on this firmware")]
    Unsupported {
        /// Jensen command id.
        command: u16,
    },

    /// The catalog count and the parsed count disagree.
    #[error("listing parsed {parsed} recordings and the recorder count was {expected}")]
    ListingMismatch {
        /// Count from the recorder.
        expected: u32,
        /// Records parsed from the stream.
        parsed: u32,
    },

    /// A download stopped receiving bytes before the known length.
    #[error("download stalled after {received} of {expected} bytes")]
    TransferStalled {
        /// Bytes accepted before the stall.
        received: u64,
        /// Bytes the listing promised.
        expected: u64,
    },

    /// The USB stack rejected setup or a transfer.
    #[error("usb error: {message}")]
    Usb {
        /// Text from the USB stack. It does not include recording bytes.
        message: String,
    },

    /// The recording has no bytes.
    #[error("the recording is empty")]
    EmptyRecording,

    /// The file is above the input cap.
    #[error("the recording is {bytes} bytes and the limit is {limit}")]
    RecordingTooLarge {
        /// Size of the file.
        bytes: u64,
        /// The crate limit.
        limit: u64,
    },

    /// Decoded audio is longer than one hour.
    #[error("the recording is longer than one hour")]
    RecordingTooLong,

    /// The bytes are not Opus, MP3, or WAV.
    #[error("the recording format is not Opus, MP3, or WAV")]
    AudioFormat,

    /// The container or codec failed.
    #[error("the recording could not be decoded: {0}")]
    Decode(String),

    /// Speaker separation failed.
    #[error("speaker separation failed: {0}")]
    Speakers(String),

    /// The speech model failed.
    #[error("transcription failed: {0}")]
    Transcript(String),

    /// A transcript file could not be written.
    #[error("could not write {path}: {message}")]
    Export {
        /// Destination path.
        path: String,
        /// The filesystem or encoder error. It does not include the transcript.
        message: String,
    },
}

impl Error {
    pub(crate) fn usb(err: impl fmt::Display) -> Self {
        Self::Usb {
            message: err.to_string(),
        }
    }
}
