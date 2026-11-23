//! SDK for HiDock recorders.
//!
//! The crate speaks the Jensen protocol on bulk endpoints `0x01` and `0x82`.
//! Open a recorder with [`Device::open_first`] from a Tokio runtime.
//! Decode a downloaded file with [`decode_recording`], then call
//! [`separate_speakers`] or [`transcribe`].
//!
//! A protocol fault poisons the session. Drop the [`Device`] and reconnect the
//! recorder before you send another command.
//!
//! ```no_run
//! # async fn demo() -> highdock_rs::Result<()> {
//! let mut device = highdock_rs::Device::open_first().await?;
//! let info = device.device_info().await?;
//! let recordings = device.list_recordings().await?;
//! let _ = (info.serial, recordings.len());
//! # Ok(())
//! # }
//! ```

mod audio;
mod codec;
mod device;
mod error;
mod frame;
mod ids;
#[cfg(test)]
mod session_tests;
mod speakers;
mod transcript;
mod transport;
mod types;
mod usb;

pub use audio::{Speech, decode_recording};
pub use device::Device;
pub use error::{Error, Result};
pub use ids::DeviceModel;
pub use speakers::{SpokenLine, TranscriptEngine, separate_speakers, transcribe};
pub use transcript::{Line, Person, TimedText, Transcript, TranscriptBook};
pub use transport::Transport;
pub use types::{
    Battery, Card, Charge, DeleteOutcome, DeviceClock, Identity, Recording, Settings, WallTime,
};
pub use usb::{UsbDeviceId, UsbTransport, list_usb_devices};
