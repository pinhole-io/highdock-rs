//! One Jensen session over a [`Transport`](crate::Transport).

use std::time::{Duration, Instant};

use crate::codec::{
    ListCursor, encode_auto_record, encode_clock, is_device_filename, parse_ack, parse_battery,
    parse_card, parse_clock, parse_delete, parse_file_count, parse_identity, parse_recording_name,
    parse_settings,
};
use crate::error::{Error, Result};
use crate::frame::{self, Frame};
use crate::ids::command::{
    DELETE_FILE, GET_BATTERY_STATUS, GET_CARD_INFO, GET_DEVICE_INFO, GET_DEVICE_TIME,
    GET_FILE_COUNT, GET_FILE_LIST, GET_RECORDING_FILE, GET_SETTINGS, SET_DEVICE_TIME, SET_SETTINGS,
    TRANSFER_FILE,
};
use crate::ids::{
    DeviceModel, H1E_FILE_CHUNK, MAX_BODY_LEN, MAX_LISTING, MAX_RECORDING_LEN, USB_READ_BYTES,
};
use crate::transport::Transport;
use crate::types::{
    Battery, Card, DeleteOutcome, DeviceClock, Identity, Recording, Settings, WallTime,
};
use crate::usb::{UsbTransport, list_usb_devices};

const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const DEFAULT_STALL_TIMEOUT: Duration = Duration::from_secs(120);

/// A claimed recorder.
///
/// Commands run one at a time. After a protocol fault, every method returns
/// [`Error::Poisoned`]. Drop this value and reconnect the recorder.
pub struct Device<T: Transport> {
    transport: T,
    model: DeviceModel,
    sequence: u32,
    carry: Vec<u8>,
    poisoned: bool,
    version_number: Option<u32>,
    command_timeout: Duration,
    stall_timeout: Duration,
}

impl Device<UsbTransport> {
    /// Claim the first connected HiDock recorder.
    pub async fn open_first() -> Result<Self> {
        let Some(id) = list_usb_devices().await?.into_iter().next() else {
            return Err(Error::NotFound);
        };
        Self::open(&id).await
    }

    /// Claim one recorder from [`list_usb_devices`].
    pub async fn open(id: &crate::UsbDeviceId) -> Result<Self> {
        let transport = UsbTransport::open(id).await?;
        Ok(Self::new(transport, id.model))
    }
}

impl<T: Transport> Device<T> {
    /// Wrap a transport that is already the exclusive owner of the pipes.
    pub fn new(transport: T, model: DeviceModel) -> Self {
        Self {
            transport,
            model,
            sequence: 0,
            carry: Vec::new(),
            poisoned: false,
            version_number: None,
            command_timeout: DEFAULT_COMMAND_TIMEOUT,
            stall_timeout: DEFAULT_STALL_TIMEOUT,
        }
    }

    /// Recorder family from the USB product id.
    pub fn model(&self) -> DeviceModel {
        self.model
    }

    /// `true` after a protocol fault.
    pub fn is_poisoned(&self) -> bool {
        self.poisoned
    }

    /// Borrow the transport. The session still owns it.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Wait for one simple reply. The default is 10 seconds.
    pub fn set_command_timeout(&mut self, timeout: Duration) {
        self.command_timeout = timeout;
    }

    /// Silence that ends a listing or a download. The default is 120 seconds.
    pub fn set_stall_timeout(&mut self, timeout: Duration) {
        self.stall_timeout = timeout;
    }

    /// Read the firmware version and the recorder serial.
    pub async fn device_info(&mut self) -> Result<Identity> {
        let frame = self.exchange(GET_DEVICE_INFO, &[]).await?;
        let identity = parse_identity(self.model, &frame.body)?;
        self.version_number = Some(identity.version_number);
        Ok(identity)
    }

    /// Read the recorder clock.
    pub async fn device_time(&mut self) -> Result<DeviceClock> {
        let frame = self.exchange(GET_DEVICE_TIME, &[]).await?;
        parse_clock(&frame.body)
    }

    /// Set the recorder clock.
    pub async fn set_time(&mut self, time: WallTime) -> Result<()> {
        let frame = self.exchange(SET_DEVICE_TIME, &encode_clock(time)).await?;
        parse_ack(SET_DEVICE_TIME, &frame.body)
    }

    /// Read auto-record, auto-play, and the optional tone flags.
    pub async fn settings(&mut self) -> Result<Settings> {
        let frame = self.exchange(GET_SETTINGS, &[]).await?;
        parse_settings(&frame.body)
    }

    /// Set auto-record.
    ///
    /// The body is two words. The auto-record byte is `1` or `2`. The second
    /// word is zero so the recorder keeps auto-play. Off is `2`, not `0`.
    pub async fn set_auto_record(&mut self, enabled: bool) -> Result<()> {
        let frame = self
            .exchange(SET_SETTINGS, &encode_auto_record(enabled))
            .await?;
        parse_ack(SET_SETTINGS, &frame.body)
    }

    /// Read card free space and capacity.
    pub async fn card(&mut self) -> Result<Card> {
        let frame = self.exchange(GET_CARD_INFO, &[]).await?;
        parse_card(&frame.body)
    }

    /// Read the battery byte and the optional voltage word.
    pub async fn battery(&mut self) -> Result<Battery> {
        let frame = self.exchange(GET_BATTERY_STATUS, &[]).await?;
        parse_battery(&frame.body)
    }

    /// Name of the file the recorder is writing now.
    ///
    /// `Ok(None)` means the recorder is idle.
    pub async fn recording_name(&mut self) -> Result<Option<String>> {
        let frame = self.exchange(GET_RECORDING_FILE, &[]).await?;
        parse_recording_name(&frame.body)
    }

    /// List stored recordings.
    ///
    /// An empty card returns no catalog command. A mismatch while more catalog
    /// frames may still arrive poisons the session.
    pub async fn list_recordings(&mut self) -> Result<Vec<Recording>> {
        let count = self.file_count().await?;
        if count == 0 {
            return Ok(Vec::new());
        }
        let sequence = self.submit(GET_FILE_LIST, &[]).await?;
        let mut cursor = ListCursor::new(self.model);
        let max_frames = (count as usize).saturating_mul(64);
        for _ in 0..max_frames {
            let frame = self
                .next_matching(GET_FILE_LIST, sequence, self.stall_timeout)
                .await?;
            if frame.body.is_empty() {
                return self.finish_listing(cursor, count);
            }
            cursor.push(&frame.body).map_err(|err| self.fail(err))?;
            if let Some(header) = cursor.header_count()
                && header != count
            {
                return Err(self.fail(Error::ListingMismatch {
                    expected: count,
                    parsed: cursor.files().len() as u32,
                }));
            }
            if cursor.files().len() > count as usize {
                return Err(self.fail(Error::ListingMismatch {
                    expected: count,
                    parsed: cursor.files().len() as u32,
                }));
            }
            if cursor.files().len() == count as usize && cursor.tail_len() == 0 {
                return Ok(cursor.files().to_vec());
            }
        }
        Err(self.fail(Error::ListingMismatch {
            expected: count,
            parsed: cursor.files().len() as u32,
        }))
    }

    /// Copy one recording.
    ///
    /// `on_chunk` must not call this device. A callback error drains the rest
    /// of the file when the recorder keeps sending, then returns that error.
    /// The session stays usable when the drain reaches the known length.
    ///
    /// On H1E firmware 6.3, a file whose last chunk is 8180 bytes also waits
    /// for an empty end frame. This method reads the identity first when the
    /// version is not known yet.
    pub async fn download<F>(&mut self, recording: &Recording, mut on_chunk: F) -> Result<()>
    where
        F: FnMut(&[u8]) -> Result<()>,
    {
        self.prepare_transfer(recording).await?;
        let expected = u64::from(recording.bytes);
        let sequence = self
            .submit(TRANSFER_FILE, recording.name.as_bytes())
            .await?;
        let mut received = 0u64;
        let mut callback_error: Option<Error> = None;
        loop {
            let frame = match self
                .next_matching(TRANSFER_FILE, sequence, self.stall_timeout)
                .await
            {
                Ok(frame) => frame,
                Err(err) => return Err(self.fail(err)),
            };
            if frame.body.is_empty() {
                if received == expected {
                    break;
                }
                return Err(self.fail(Error::TransferStalled { received, expected }));
            }
            let next = received + frame.body.len() as u64;
            if next > expected {
                return Err(self.fail(Error::Malformed {
                    command: TRANSFER_FILE,
                }));
            }
            if callback_error.is_none()
                && let Err(error) = on_chunk(&frame.body)
            {
                callback_error = Some(error);
            }
            received = next;
            if received == expected {
                if self.needs_h1e_eof(frame.body.len()) {
                    self.read_h1e_eof(sequence).await?;
                }
                break;
            }
        }
        match callback_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    /// Delete one recording that [`Device::list_recordings`] returned.
    pub async fn delete_recording(&mut self, recording: &Recording) -> Result<DeleteOutcome> {
        self.require_name(&recording.name)?;
        let frame = self
            .exchange(DELETE_FILE, recording.name.as_bytes())
            .await?;
        parse_delete(&frame.body)
    }

    fn finish_listing(&self, cursor: ListCursor, count: u32) -> Result<Vec<Recording>> {
        if cursor.files().len() as u32 != count || cursor.tail_len() != 0 {
            return Err(Error::ListingMismatch {
                expected: count,
                parsed: cursor.files().len() as u32,
            });
        }
        Ok(cursor.files().to_vec())
    }

    async fn prepare_transfer(&mut self, recording: &Recording) -> Result<()> {
        self.ensure()?;
        self.require_name(&recording.name)?;
        if recording.bytes > MAX_RECORDING_LEN {
            return Err(Error::InvalidLength {
                length: u64::from(recording.bytes),
            });
        }
        if self.version_number.is_none() {
            self.device_info().await?;
        }
        Ok(())
    }

    fn require_name(&self, name: &str) -> Result<()> {
        if is_device_filename(name, self.model) {
            Ok(())
        } else {
            Err(Error::InvalidFileName)
        }
    }

    fn needs_h1e_eof(&self, last_chunk: usize) -> bool {
        let Some(version) = self.version_number else {
            return false;
        };
        let major = (version >> 16) & 0xff;
        let minor = (version >> 8) & 0xff;
        self.model == DeviceModel::H1e && last_chunk == H1E_FILE_CHUNK && major == 6 && minor == 3
    }

    async fn read_h1e_eof(&mut self, sequence: u32) -> Result<()> {
        let frame = self
            .next_matching(TRANSFER_FILE, sequence, self.stall_timeout)
            .await?;
        if frame.body.is_empty() {
            Ok(())
        } else {
            Err(self.fail(Error::Malformed {
                command: TRANSFER_FILE,
            }))
        }
    }

    async fn file_count(&mut self) -> Result<u32> {
        let frame = self.exchange(GET_FILE_COUNT, &[]).await?;
        let count = parse_file_count(&frame.body)?;
        if count > MAX_LISTING {
            return Err(Error::ListingMismatch {
                expected: count,
                parsed: 0,
            });
        }
        Ok(count)
    }

    async fn exchange(&mut self, command_id: u16, body: &[u8]) -> Result<Frame> {
        let sequence = self.submit(command_id, body).await?;
        self.next_matching(command_id, sequence, self.command_timeout)
            .await
    }

    async fn submit(&mut self, command_id: u16, body: &[u8]) -> Result<u32> {
        self.ensure()?;
        let sequence = self.sequence;
        let bytes = frame::encode(command_id, sequence, body)?;
        self.sequence = self.sequence.wrapping_add(1);
        self.transport
            .write(&bytes, self.command_timeout)
            .await
            .map_err(|err| self.fail(err))?;
        Ok(sequence)
    }

    async fn next_matching(
        &mut self,
        command_id: u16,
        sequence: u32,
        timeout: Duration,
    ) -> Result<Frame> {
        let frame = self.next_frame(command_id, timeout).await?;
        if frame.command == command_id && frame.sequence == sequence {
            return Ok(frame);
        }
        let err = Error::UnexpectedReply {
            got: frame.command,
            got_sequence: frame.sequence,
            expected: command_id,
            expected_sequence: sequence,
        };
        Err(self.fail(err))
    }

    async fn next_frame(&mut self, command_id: u16, timeout: Duration) -> Result<Frame> {
        let deadline = Instant::now() + timeout;
        loop {
            self.ensure()?;
            match frame::decode(&self.carry) {
                Ok(Some((frame, used))) => {
                    self.carry.drain(..used);
                    return Ok(frame);
                }
                Ok(None) => {}
                Err(err) => return Err(self.fail(err)),
            }
            if self.carry.len() > 12 + MAX_BODY_LEN + 255 {
                return Err(self.fail(Error::FrameTooLarge {
                    max: 12 + MAX_BODY_LEN + 255,
                }));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(self.fail(Error::Timeout {
                    command: command_id,
                }));
            }
            let mut buf = vec![0u8; USB_READ_BYTES];
            match self.transport.read(&mut buf, left).await {
                Ok(0) => {}
                Ok(n) => self.carry.extend_from_slice(&buf[..n]),
                Err(Error::IoTimeout) => {
                    return Err(self.fail(Error::Timeout {
                        command: command_id,
                    }));
                }
                Err(err) => return Err(self.fail(err)),
            }
        }
    }

    fn ensure(&self) -> Result<()> {
        if self.poisoned {
            Err(Error::Poisoned)
        } else {
            Ok(())
        }
    }

    fn fail(&mut self, error: Error) -> Error {
        self.poisoned = true;
        error
    }
}
