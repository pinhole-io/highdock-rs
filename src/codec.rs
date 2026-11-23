//! Payload parsers for the commands this crate sends.

use crate::error::{Error, Result};
use crate::ids::DeviceModel;
use crate::ids::command;
use crate::types::{
    Battery, Card, Charge, DeleteOutcome, DeviceClock, Identity, Recording, Settings, WallTime,
};

pub(crate) fn parse_identity(model: DeviceModel, body: &[u8]) -> Result<Identity> {
    if !(17..=20).contains(&body.len()) {
        return Err(Error::Malformed {
            command: command::GET_DEVICE_INFO,
        });
    }
    if model == DeviceModel::H1e && body[17..].iter().any(|byte| *byte != 0) {
        return Err(Error::Malformed {
            command: command::GET_DEVICE_INFO,
        });
    }
    let serial_end = if model == DeviceModel::H1e {
        17
    } else {
        body.len()
    };
    let serial = c_string(&body[4..serial_end]).map_err(|_| Error::Malformed {
        command: command::GET_DEVICE_INFO,
    })?;
    if serial.is_empty() {
        return Err(Error::Malformed {
            command: command::GET_DEVICE_INFO,
        });
    }
    let version_number = u32::from_be_bytes([body[0], body[1], body[2], body[3]]);
    Ok(Identity {
        model,
        version_number,
        version: format!("{}.{}.{}", body[1], body[2], body[3]),
        serial,
    })
}

pub(crate) fn parse_clock(body: &[u8]) -> Result<DeviceClock> {
    if body.len() != 7 || body.iter().any(|byte| !bcd_byte(*byte)) {
        return Err(Error::Malformed {
            command: command::GET_DEVICE_TIME,
        });
    }
    if body.iter().all(|byte| *byte == 0) {
        return Ok(DeviceClock::Unknown);
    }
    let digits: String = body.iter().map(|byte| format!("{byte:02x}")).collect();
    let year: u16 = digits[0..4].parse().map_err(|_| malformed_time())?;
    let month: u8 = digits[4..6].parse().map_err(|_| malformed_time())?;
    let day: u8 = digits[6..8].parse().map_err(|_| malformed_time())?;
    let hour: u8 = digits[8..10].parse().map_err(|_| malformed_time())?;
    let minute: u8 = digits[10..12].parse().map_err(|_| malformed_time())?;
    let second: u8 = digits[12..14].parse().map_err(|_| malformed_time())?;
    Ok(DeviceClock::Set(WallTime::new(
        year, month, day, hour, minute, second,
    )?))
}

pub(crate) fn encode_clock(time: WallTime) -> [u8; 7] {
    let text = format!(
        "{:04}{:02}{:02}{:02}{:02}{:02}",
        time.year(),
        time.month(),
        time.day(),
        time.hour(),
        time.minute(),
        time.second()
    );
    let bytes = text.as_bytes();
    let mut out = [0u8; 7];
    for (index, slot) in out.iter_mut().enumerate() {
        let high = bytes[index * 2] - b'0';
        let low = bytes[index * 2 + 1] - b'0';
        *slot = (high << 4) | low;
    }
    out
}

pub(crate) fn parse_file_count(body: &[u8]) -> Result<u32> {
    let bytes: [u8; 4] = body.try_into().map_err(|_| Error::Malformed {
        command: command::GET_FILE_COUNT,
    })?;
    Ok(u32::from_be_bytes(bytes))
}

pub(crate) fn parse_ack(command_id: u16, body: &[u8]) -> Result<()> {
    if body.len() != 1 || body[0] != 0 {
        return Err(Error::Malformed {
            command: command_id,
        });
    }
    Ok(())
}

pub(crate) fn parse_delete(body: &[u8]) -> Result<DeleteOutcome> {
    match body {
        [0] => Ok(DeleteOutcome::Deleted),
        [1] => Ok(DeleteOutcome::Missing),
        _ => Err(Error::Malformed {
            command: command::DELETE_FILE,
        }),
    }
}

pub(crate) fn parse_settings(body: &[u8]) -> Result<Settings> {
    // A P1 can return a fifth word. The first four words keep the same offsets.
    if !matches!(body.len(), 8 | 12 | 16 | 20) {
        return Err(Error::Malformed {
            command: command::GET_SETTINGS,
        });
    }
    Ok(Settings {
        auto_record: body[3] == 1,
        auto_play: body[7] == 1,
        notification: (body.len() >= 12).then(|| body[11] == 1),
        bluetooth_tone: (body.len() >= 16).then(|| body[15] != 1),
    })
}

pub(crate) fn encode_auto_record(enabled: bool) -> [u8; 8] {
    [0, 0, 0, if enabled { 1 } else { 2 }, 0, 0, 0, 0]
}

pub(crate) fn parse_card(body: &[u8]) -> Result<Card> {
    if !(12..=28).contains(&body.len()) {
        return Err(Error::Malformed {
            command: command::GET_CARD_INFO,
        });
    }
    let free_mib = u32::from_be_bytes([body[0], body[1], body[2], body[3]]);
    let capacity_mib = u32::from_be_bytes([body[4], body[5], body[6], body[7]]);
    let status_raw = u32::from_be_bytes([body[8], body[9], body[10], body[11]]);
    if free_mib > capacity_mib {
        return Err(Error::Malformed {
            command: command::GET_CARD_INFO,
        });
    }
    Ok(Card {
        free_mib,
        capacity_mib,
        status_raw,
    })
}

pub(crate) fn parse_battery(body: &[u8]) -> Result<Battery> {
    if body.len() != 2 && body.len() != 6 {
        return Err(Error::Malformed {
            command: command::GET_BATTERY_STATUS,
        });
    }
    let state = match body[0] {
        0 => Charge::Idle,
        1 => Charge::Charging,
        2 => Charge::Full,
        other => Charge::Other(other),
    };
    let voltage_raw =
        (body.len() == 6).then(|| u32::from_be_bytes([body[2], body[3], body[4], body[5]]));
    Ok(Battery {
        state,
        level: body[1],
        voltage_raw,
    })
}

pub(crate) fn parse_recording_name(body: &[u8]) -> Result<Option<String>> {
    if body.is_empty() {
        return Ok(None);
    }
    let name = c_string(body).map_err(|_| Error::Malformed {
        command: command::GET_RECORDING_FILE,
    })?;
    if !is_device_filename(&name, DeviceModel::Unknown) {
        return Err(Error::Malformed {
            command: command::GET_RECORDING_FILE,
        });
    }
    Ok(Some(name))
}

/// Incremental catalog parser. Feed each command-4 body in order.
pub(crate) struct ListCursor {
    model: DeviceModel,
    files: Vec<Recording>,
    header_count: Option<u32>,
    header_done: bool,
    tail: Vec<u8>,
}

impl ListCursor {
    pub(crate) fn new(model: DeviceModel) -> Self {
        Self {
            model,
            files: Vec::new(),
            header_count: None,
            header_done: false,
            tail: Vec::new(),
        }
    }

    pub(crate) fn files(&self) -> &[Recording] {
        &self.files
    }

    pub(crate) fn header_count(&self) -> Option<u32> {
        self.header_count
    }

    pub(crate) fn tail_len(&self) -> usize {
        self.tail.len()
    }

    pub(crate) fn push(&mut self, chunk: &[u8]) -> Result<()> {
        let mut work = Vec::with_capacity(self.tail.len() + chunk.len());
        work.extend_from_slice(&self.tail);
        work.extend_from_slice(chunk);
        let mut pos = 0usize;
        if !self.header_done {
            if work.len() < 2 {
                self.tail = work;
                return Ok(());
            }
            if work[0] == 0xff && work[1] == 0xff {
                if work.len() < 6 {
                    self.tail = work;
                    return Ok(());
                }
                self.header_count = Some(u32::from_be_bytes([work[2], work[3], work[4], work[5]]));
                pos = 6;
            }
            self.header_done = true;
        }
        while pos + 4 <= work.len() {
            let name_len =
                u32::from_be_bytes([0, work[pos + 1], work[pos + 2], work[pos + 3]]) as usize;
            if !(1..=32).contains(&name_len) {
                return Err(Error::Malformed {
                    command: command::GET_FILE_LIST,
                });
            }
            let entry_len = 4 + name_len + 4 + 6 + 16;
            if pos + entry_len > work.len() {
                break;
            }
            let version = work[pos];
            let name = recording_name(&work[pos + 4..pos + 4 + name_len], self.model)?;
            let length_at = pos + 4 + name_len;
            let bytes = u32::from_be_bytes([
                work[length_at],
                work[length_at + 1],
                work[length_at + 2],
                work[length_at + 3],
            ]);
            let sig_at = length_at + 10;
            let mut signature = [0u8; 16];
            signature.copy_from_slice(&work[sig_at..sig_at + 16]);
            if self.files.iter().any(|file| file.name == name) {
                return Err(Error::Malformed {
                    command: command::GET_FILE_LIST,
                });
            }
            self.files.push(Recording {
                name,
                bytes,
                version,
                signature,
            });
            pos += entry_len;
        }
        self.tail = work[pos..].to_vec();
        Ok(())
    }
}

impl Recording {
    /// Build a recording handle from a file name and a length.
    ///
    /// The catalog fields `version` and `signature` stay zero.
    /// [`crate::Device::list_recordings`] fills them from the recorder.
    pub fn new(name: impl Into<String>, bytes: u32) -> Result<Self> {
        let name = name.into();
        if !is_device_filename(&name, DeviceModel::Unknown) {
            return Err(Error::InvalidFileName);
        }
        if bytes > crate::ids::MAX_RECORDING_LEN {
            return Err(Error::InvalidLength {
                length: u64::from(bytes),
            });
        }
        Ok(Self {
            name,
            bytes,
            version: 0,
            signature: [0; 16],
        })
    }
}

pub(crate) fn is_device_filename(name: &str, model: DeviceModel) -> bool {
    let bytes = name.as_bytes();
    if bytes.is_empty() || bytes.len() > model.max_filename_len() {
        return false;
    }
    if name.trim().is_empty() || name == "." || name == ".." {
        return false;
    }
    bytes.iter().all(|byte| {
        (0x20..=0x7e).contains(byte) && *byte != b'\\' && *byte != b'/' && *byte != b':'
    })
}

fn recording_name(bytes: &[u8], model: DeviceModel) -> Result<String> {
    let name = c_string(bytes).map_err(|_| Error::Malformed {
        command: command::GET_FILE_LIST,
    })?;
    if !is_device_filename(&name, model) {
        return Err(Error::Malformed {
            command: command::GET_FILE_LIST,
        });
    }
    Ok(name)
}

fn c_string(bytes: &[u8]) -> std::result::Result<String, ()> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    if bytes[end..].iter().any(|byte| *byte != 0) {
        return Err(());
    }
    let text = &bytes[..end];
    if !text.is_ascii() {
        return Err(());
    }
    Ok(text.iter().map(|byte| char::from(*byte)).collect())
}

fn bcd_byte(byte: u8) -> bool {
    let high = byte >> 4;
    let low = byte & 0x0f;
    high <= 9 && low <= 9
}

fn malformed_time() -> Error {
    Error::Malformed {
        command: command::GET_DEVICE_TIME,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info_body(serial: &str) -> Vec<u8> {
        let mut body = vec![0x00, 0x06, 0x03, 0x00];
        body.extend_from_slice(serial.as_bytes());
        body.resize(17, 0);
        body
    }

    #[test]
    fn identity_reads_h1e_version_and_serial() {
        let identity = parse_identity(DeviceModel::H1e, &info_body("ABC123")).unwrap();
        assert_eq!(identity.version, "6.3.0");
        assert_eq!(identity.version_number, 393_984);
        assert_eq!(identity.serial, "ABC123");
    }

    #[test]
    fn h1e_rejects_nonzero_bytes_after_the_serial_field() {
        let mut body = info_body("ABC123");
        body.push(1);
        assert!(parse_identity(DeviceModel::H1e, &body).is_err());
    }

    #[test]
    fn clock_decodes_bcd_and_the_zero_sentinel() {
        let clock = parse_clock(&[0x20, 0x26, 0x11, 0x23, 0x08, 0x04, 0x05]).unwrap();
        let DeviceClock::Set(time) = clock else {
            panic!("expected a set clock");
        };
        assert_eq!(
            (
                time.year(),
                time.month(),
                time.day(),
                time.hour(),
                time.minute(),
                time.second()
            ),
            (2026, 11, 23, 8, 4, 5)
        );
        assert_eq!(parse_clock(&[0; 7]).unwrap(), DeviceClock::Unknown);
        assert!(parse_clock(&[0x20, 0x26, 0x11, 0x23, 0x08, 0x04, 0x1a]).is_err());
    }

    #[test]
    fn wall_time_rejects_a_missing_leap_day() {
        assert!(WallTime::new(2024, 2, 29, 0, 0, 0).is_ok());
        assert!(WallTime::new(2023, 2, 29, 0, 0, 0).is_err());
        assert!(WallTime::new(1900, 2, 29, 0, 0, 0).is_err());
        assert!(WallTime::new(2000, 2, 29, 0, 0, 0).is_ok());
    }

    #[test]
    fn settings_card_and_battery_match_the_reference_layout() {
        let settings = parse_settings(&[0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0]).unwrap();
        assert!(settings.auto_record);
        assert!(!settings.auto_play);
        assert_eq!(settings.notification, Some(true));
        assert_eq!(settings.bluetooth_tone, Some(true));
        assert_eq!(encode_auto_record(false), [0, 0, 0, 2, 0, 0, 0, 0]);
        let p1 =
            parse_settings(&[0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 2]).unwrap();
        assert!(p1.auto_record);
        assert!(!p1.auto_play);
        assert_eq!(p1.notification, Some(true));
        assert_eq!(p1.bluetooth_tone, Some(true));

        let card = parse_card(&{
            let mut body = Vec::new();
            body.extend_from_slice(&10u32.to_be_bytes());
            body.extend_from_slice(&64u32.to_be_bytes());
            body.extend_from_slice(&0u32.to_be_bytes());
            body
        })
        .unwrap();
        assert_eq!(card.used_mib(), 54);

        let battery = parse_battery(&[1, 80, 0, 0, 0x10, 0x68]).unwrap();
        assert_eq!(battery.state, Charge::Charging);
        assert_eq!(battery.level, 80);
        assert_eq!(battery.voltage_raw, Some(0x1068));
    }

    fn entry(name: &str, len: u32) -> Vec<u8> {
        let mut body = vec![5, 0, 0, name.len() as u8];
        body.extend_from_slice(name.as_bytes());
        body.extend_from_slice(&len.to_be_bytes());
        body.extend_from_slice(&[0; 6]);
        body.extend_from_slice(&[0x11; 16]);
        body
    }

    #[test]
    fn listing_accepts_a_header_split_across_pushes() {
        let mut cursor = ListCursor::new(DeviceModel::H1e);
        let mut first = vec![0xff, 0xff];
        first.extend_from_slice(&2u32.to_be_bytes());
        first.extend_from_slice(&entry("a.hda", 4));
        let split = 8;
        cursor.push(&first[..split]).unwrap();
        cursor.push(&first[split..]).unwrap();
        cursor.push(&entry("b.hda", 8)).unwrap();
        assert_eq!(cursor.header_count(), Some(2));
        assert_eq!(cursor.files().len(), 2);
        assert_eq!(cursor.files()[1].bytes, 8);
        assert_eq!(cursor.tail_len(), 0);
    }

    #[test]
    fn recording_new_rejects_a_path() {
        assert!(Recording::new("../note.hda", 4).is_err());
        assert_eq!(Recording::new("note.hda", 4).unwrap().bytes, 4);
    }

    #[test]
    fn listing_rejects_a_name_length_over_32() {
        let mut cursor = ListCursor::new(DeviceModel::P1);
        let err = cursor.push(&[1, 0, 0, 33]).unwrap_err();
        assert!(matches!(err, Error::Malformed { .. }));
    }
}
