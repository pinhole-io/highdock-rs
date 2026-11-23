//! Values decoded from recorder payloads.

use crate::error::{Error, Result};
use crate::ids::DeviceModel;

/// Identity from command 1.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Identity {
    /// Recorder family from the USB product id.
    pub model: DeviceModel,
    /// Four header bytes, big-endian.
    pub version_number: u32,
    /// `major.minor.patch` from header bytes 1, 2, and 3.
    pub version: String,
    /// Recorder serial. This is not the USB string descriptor.
    pub serial: String,
}

/// One stored recording from the file list.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Recording {
    /// File name on the recorder.
    pub name: String,
    /// File length in bytes.
    pub bytes: u32,
    /// File-format byte from the catalog entry.
    pub version: u8,
    /// 16-byte signature from the catalog entry.
    pub signature: [u8; 16],
}

/// Charge state from command 4100.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Charge {
    /// Not charging.
    Idle,
    /// Charging.
    Charging,
    /// Full.
    Full,
    /// A status byte this crate does not name.
    Other(u8),
}

/// Battery report.
///
/// `level` is the raw device byte. This crate does not scale it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Battery {
    /// Charge state.
    pub state: Charge,
    /// Raw level byte.
    pub level: u8,
    /// Optional raw voltage word. The unit is the recorder's unit.
    pub voltage_raw: Option<u32>,
}

/// Storage card report.
///
/// The reference client treats the sizes as mebibytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Card {
    /// Free space.
    pub free_mib: u32,
    /// Card capacity.
    pub capacity_mib: u32,
    /// Status word from the recorder, unparsed.
    pub status_raw: u32,
}

impl Card {
    /// Capacity minus free space.
    pub fn used_mib(&self) -> u32 {
        self.capacity_mib.saturating_sub(self.free_mib)
    }
}

/// Settings from command 11.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Settings {
    /// Auto-record flag.
    pub auto_record: bool,
    /// Auto-play flag.
    pub auto_play: bool,
    /// Present when the body is at least 12 bytes.
    pub notification: Option<bool>,
    /// Present when the body is at least 16 bytes.
    ///
    /// The reference client treats byte 15 as off when the value is `1`.
    pub bluetooth_tone: Option<bool>,
}

/// Result of deleting one recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeleteOutcome {
    /// The recorder deleted the file.
    Deleted,
    /// The recorder reported that the file is absent.
    Missing,
}

/// Civil time used by the recorder clock.
///
/// Fields stay private so a value from [`WallTime::new`] keeps a real date.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WallTime {
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    minute: u8,
    second: u8,
}

impl WallTime {
    /// Build a clock value the recorder can store.
    ///
    /// The year is `100..=9999`. The date must exist, including leap days.
    pub fn new(year: u16, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> Result<Self> {
        if !(100..=9999).contains(&year)
            || !(1..=12).contains(&month)
            || hour > 23
            || minute > 59
            || second > 59
        {
            return Err(Error::InvalidTime);
        }
        if day == 0 || day > days_in_month(year, month) {
            return Err(Error::InvalidTime);
        }
        Ok(Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
        })
    }

    /// Year, `100..=9999`.
    pub fn year(self) -> u16 {
        self.year
    }

    /// Month, `1..=12`.
    pub fn month(self) -> u8 {
        self.month
    }

    /// Day of the month.
    pub fn day(self) -> u8 {
        self.day
    }

    /// Hour, `0..=23`.
    pub fn hour(self) -> u8 {
        self.hour
    }

    /// Minute, `0..=59`.
    pub fn minute(self) -> u8 {
        self.minute
    }

    /// Second, `0..=59`.
    pub fn second(self) -> u8 {
        self.second
    }
}

/// Recorder clock from command 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeviceClock {
    /// The recorder returned an all-zero BCD time.
    Unknown,
    /// A validated civil time.
    Set(WallTime),
}

fn days_in_month(year: u16, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap(year: u16) -> bool {
    let year = u32::from(year);
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}
