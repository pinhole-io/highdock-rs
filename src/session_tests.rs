use std::collections::VecDeque;
use std::time::Duration;

use crate::frame;
use crate::ids::DeviceModel;
use crate::ids::command::{
    DELETE_FILE, GET_DEVICE_INFO, GET_FILE_COUNT, GET_FILE_LIST, GET_SETTINGS, SET_DEVICE_TIME,
    TRANSFER_FILE,
};
use crate::transport::Transport;
use crate::types::{DeleteOutcome, DeviceClock, Recording, WallTime};
use crate::{Device, Error};

struct Script {
    inbound: VecDeque<Vec<u8>>,
    outbound: Vec<Vec<u8>>,
}

impl Script {
    fn new(inbound: Vec<Vec<u8>>) -> Self {
        Self {
            inbound: inbound.into(),
            outbound: Vec::new(),
        }
    }
}

impl Transport for Script {
    async fn write(&mut self, data: &[u8], _timeout: Duration) -> crate::Result<()> {
        self.outbound.push(data.to_vec());
        Ok(())
    }

    async fn read(&mut self, buf: &mut [u8], _timeout: Duration) -> crate::Result<usize> {
        let Some(chunk) = self.inbound.pop_front() else {
            return Err(Error::Disconnected);
        };
        let n = chunk.len().min(buf.len());
        buf[..n].copy_from_slice(&chunk[..n]);
        if n < chunk.len() {
            self.inbound.push_front(chunk[n..].to_vec());
        }
        Ok(n)
    }
}

fn reply(command: u16, sequence: u32, body: &[u8]) -> Vec<u8> {
    frame::encode(command, sequence, body).unwrap()
}

fn info_body(major: u8, minor: u8, patch: u8) -> Vec<u8> {
    let mut body = vec![0, major, minor, patch];
    body.extend_from_slice(b"SN1");
    body.resize(17, 0);
    body
}

fn entry(name: &str, len: u32) -> Vec<u8> {
    let mut body = vec![1, 0, 0, name.len() as u8];
    body.extend_from_slice(name.as_bytes());
    body.extend_from_slice(&len.to_be_bytes());
    body.extend_from_slice(&[0; 6]);
    body.extend_from_slice(&[9; 16]);
    body
}

fn recording(name: &str, bytes: u32) -> Recording {
    Recording {
        name: name.to_owned(),
        bytes,
        version: 1,
        signature: [9; 16],
    }
}

fn device(model: DeviceModel, inbound: Vec<Vec<u8>>) -> Device<Script> {
    Device::new(Script::new(inbound), model)
}

#[tokio::test]
async fn device_info_reads_version_and_serial() {
    let mut device = device(
        DeviceModel::H1e,
        vec![reply(GET_DEVICE_INFO, 0, &info_body(6, 3, 0))],
    );
    let info = device.device_info().await.unwrap();
    assert_eq!(info.version, "6.3.0");
    assert_eq!(info.version_number, 393_984);
    assert_eq!(info.serial, "SN1");
    assert!(!device.is_poisoned());
}

#[tokio::test]
async fn a_bad_magic_poisons_the_next_command() {
    let mut device = device(
        DeviceModel::P1,
        vec![vec![0x00, 0x11, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]],
    );
    assert!(device.device_info().await.is_err());
    assert!(device.is_poisoned());
    assert!(matches!(device.battery().await, Err(Error::Poisoned)));
}

#[tokio::test]
async fn set_time_sends_bcd() {
    let mut device = device(DeviceModel::P1, vec![reply(SET_DEVICE_TIME, 0, &[0])]);
    let time = WallTime::new(2026, 11, 23, 8, 4, 5).unwrap();
    device.set_time(time).await.unwrap();
    let (frame, _) = frame::decode(&device.transport().outbound[0])
        .unwrap()
        .unwrap();
    assert_eq!(frame.body, [0x20, 0x26, 0x11, 0x23, 0x08, 0x04, 0x05]);
}

#[tokio::test]
async fn clock_zero_is_unknown() {
    let mut device = device(DeviceModel::P1, vec![reply(2, 0, &[0; 7])]);
    assert_eq!(device.device_time().await.unwrap(), DeviceClock::Unknown);
}

#[tokio::test]
async fn settings_sends_after_a_low_firmware_number() {
    let mut device = device(
        DeviceModel::P1,
        vec![
            reply(GET_DEVICE_INFO, 0, &info_body(1, 4, 7)),
            reply(
                GET_SETTINGS,
                1,
                &[0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 2],
            ),
        ],
    );
    device.device_info().await.unwrap();
    let settings = device.settings().await.unwrap();
    assert!(settings.auto_record);
    assert!(!settings.auto_play);
    assert_eq!(settings.notification, Some(true));
    assert_eq!(settings.bluetooth_tone, Some(true));
    assert_eq!(device.transport().outbound.len(), 2);
}

#[tokio::test]
async fn empty_catalog_stops_after_the_count() {
    let mut device = device(
        DeviceModel::P1,
        vec![reply(GET_FILE_COUNT, 0, &0u32.to_be_bytes())],
    );
    let files = device.list_recordings().await.unwrap();
    assert!(files.is_empty());
    assert_eq!(device.transport().outbound.len(), 1);
}

#[tokio::test]
async fn listing_reassembles_a_split_frame_and_a_split_record() {
    let mut listed = vec![0xff, 0xff];
    listed.extend_from_slice(&1u32.to_be_bytes());
    listed.extend_from_slice(&entry("note.hda", 12));
    let frame = reply(GET_FILE_LIST, 1, &listed);
    let mut device = device(
        DeviceModel::H1e,
        vec![
            reply(GET_FILE_COUNT, 0, &1u32.to_be_bytes()),
            frame[..10].to_vec(),
            frame[10..].to_vec(),
        ],
    );
    let files = device.list_recordings().await.unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].name, "note.hda");
    assert_eq!(files[0].bytes, 12);
}

#[tokio::test]
async fn download_stops_at_the_listed_length() {
    let file = recording("note.hda", 6);
    let mut device = device(
        DeviceModel::P1,
        vec![
            reply(GET_DEVICE_INFO, 0, &info_body(1, 2, 3)),
            reply(TRANSFER_FILE, 1, b"abcdef"),
        ],
    );
    let mut got = Vec::new();
    device
        .download(&file, |chunk| {
            got.extend_from_slice(chunk);
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(got, b"abcdef");
    let (request, _) = frame::decode(&device.transport().outbound[1])
        .unwrap()
        .unwrap();
    assert_eq!(request.body, b"note.hda");
    assert!(!device.is_poisoned());
}

#[tokio::test]
async fn h1e_6_3_waits_for_the_empty_end_frame() {
    let file = recording("note.hda", 8180);
    let mut device = device(
        DeviceModel::H1e,
        vec![
            reply(GET_DEVICE_INFO, 0, &info_body(6, 3, 4)),
            reply(TRANSFER_FILE, 1, &vec![7; 8180]),
            reply(TRANSFER_FILE, 1, &[]),
            reply(GET_DEVICE_INFO, 2, &info_body(6, 3, 4)),
        ],
    );
    device.download(&file, |_| Ok(())).await.unwrap();
    let info = device.device_info().await.unwrap();
    assert_eq!(info.version, "6.3.4");
}

#[tokio::test]
async fn older_h1e_does_not_wait_for_an_extra_end_frame() {
    let file = recording("note.hda", 8180);
    let mut device = device(
        DeviceModel::H1e,
        vec![
            reply(GET_DEVICE_INFO, 0, &info_body(5, 2, 0)),
            reply(TRANSFER_FILE, 1, &vec![7; 8180]),
            reply(GET_DEVICE_INFO, 2, &info_body(5, 2, 0)),
        ],
    );
    device.download(&file, |_| Ok(())).await.unwrap();
    assert_eq!(device.device_info().await.unwrap().version, "5.2.0");
}

#[tokio::test]
async fn a_callback_error_still_drains_the_file() {
    let file = recording("note.hda", 4);
    let mut device = device(
        DeviceModel::P1,
        vec![
            reply(GET_DEVICE_INFO, 0, &info_body(1, 0, 0)),
            reply(TRANSFER_FILE, 1, b"ab"),
            reply(TRANSFER_FILE, 1, b"cd"),
            reply(GET_FILE_COUNT, 2, &0u32.to_be_bytes()),
        ],
    );
    let err = device
        .download(&file, |_| Err(Error::InvalidFileName))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::InvalidFileName));
    assert!(!device.is_poisoned());
    assert!(device.list_recordings().await.unwrap().is_empty());
}

#[tokio::test]
async fn delete_uses_the_recording_name() {
    let file = recording("note.hda", 4);
    let mut device = device(DeviceModel::P1, vec![reply(DELETE_FILE, 0, &[0])]);
    assert_eq!(
        device.delete_recording(&file).await.unwrap(),
        DeleteOutcome::Deleted
    );
    let (frame, _) = frame::decode(&device.transport().outbound[0])
        .unwrap()
        .unwrap();
    assert_eq!(frame.body, b"note.hda");
}

#[tokio::test]
async fn an_invalid_name_does_not_touch_the_recorder() {
    let file = recording("../note.hda", 4);
    let mut device = device(DeviceModel::H1e, Vec::new());
    assert!(matches!(
        device.delete_recording(&file).await,
        Err(Error::InvalidFileName)
    ));
    assert!(device.transport().outbound.is_empty());
}

#[tokio::test]
async fn a_mismatched_reply_poisons_the_session() {
    let mut device = device(DeviceModel::P1, vec![reply(99, 0, &[0])]);
    assert!(device.device_info().await.is_err());
    assert!(device.is_poisoned());
}
