//! USB identifiers and Jensen command ids.

/// Actions Semiconductor, used by HiDock recorders.
pub const USB_VENDOR_ID: u16 = 0x10d6;

/// Vendor id seen on some later recorders.
pub const USB_ALTERNATE_VENDOR_ID: u16 = 0x3887;

/// Bulk OUT endpoint. Host to recorder.
pub const EP_OUT: u8 = 0x01;

/// Bulk IN endpoint. Recorder to host.
pub const EP_IN: u8 = 0x82;

/// Host read size.
///
/// An H1E frame can be exactly 8192 bytes and omit a short packet. A larger
/// read waits for another packet that may never arrive.
pub const USB_READ_BYTES: usize = 8192;

/// Largest body this crate will encode or accept.
pub const MAX_BODY_LEN: usize = 64 * 1024;

/// File-transfer chunk that fills an 8192-byte H1E frame (`8192 - 12`).
pub(crate) const H1E_FILE_CHUNK: usize = 8180;

/// Largest recording this crate will download.
pub(crate) const MAX_RECORDING_LEN: u32 = 0x7fff_ffff;

/// Largest catalog this crate will accept.
pub(crate) const MAX_LISTING: u32 = 8192;

/// Pause after the interface claim before the first command.
pub(crate) const CLAIM_SETTLE: std::time::Duration = std::time::Duration::from_millis(300);

/// Jensen command ids this crate sends.
pub(crate) mod command {
    pub(crate) const GET_DEVICE_INFO: u16 = 1;
    pub(crate) const GET_DEVICE_TIME: u16 = 2;
    pub(crate) const SET_DEVICE_TIME: u16 = 3;
    pub(crate) const GET_FILE_LIST: u16 = 4;
    pub(crate) const TRANSFER_FILE: u16 = 5;
    pub(crate) const GET_FILE_COUNT: u16 = 6;
    pub(crate) const DELETE_FILE: u16 = 7;
    pub(crate) const GET_SETTINGS: u16 = 11;
    pub(crate) const SET_SETTINGS: u16 = 12;
    pub(crate) const GET_CARD_INFO: u16 = 16;
    pub(crate) const GET_RECORDING_FILE: u16 = 18;
    pub(crate) const GET_BATTERY_STATUS: u16 = 4100;
}

/// Recorder family, taken from the USB product id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DeviceModel {
    /// HiDock H1.
    H1,
    /// HiDock H1E.
    H1e,
    /// HiDock P1.
    P1,
    /// HiDock P1 Mini.
    P1Mini,
    /// HiDock H1 Lite.
    H1Lite,
    /// The vendor matched and the product id is not in the known table.
    Unknown,
}

impl DeviceModel {
    /// Map a USB product id to a recorder family.
    pub fn from_product_id(product_id: u16) -> Self {
        match product_id {
            0xaf0c | 0xb00c | 0x0100 | 0x0102 => Self::H1,
            0xaf0d | 0xb00d | 0x0101 | 0x0103 => Self::H1e,
            0xaf0e | 0xb00e | 0x2040 => Self::P1,
            0xaf0f | 0xb00f | 0x2041 => Self::P1Mini,
            0x0104 => Self::H1Lite,
            _ => Self::Unknown,
        }
    }

    pub(crate) fn max_filename_len(self) -> usize {
        match self {
            Self::H1e => 27,
            _ => 32,
        }
    }
}

impl std::fmt::Display for DeviceModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::H1 => "H1",
            Self::H1e => "H1E",
            Self::P1 => "P1",
            Self::P1Mini => "P1 Mini",
            Self::H1Lite => "H1 Lite",
            Self::Unknown => "unknown",
        };
        f.write_str(name)
    }
}

pub(crate) fn is_hidock_vendor(vendor_id: u16) -> bool {
    vendor_id == USB_VENDOR_ID || vendor_id == USB_ALTERNATE_VENDOR_ID
}

pub(crate) fn is_known_product(product_id: u16) -> bool {
    !matches!(
        DeviceModel::from_product_id(product_id),
        DeviceModel::Unknown
    )
}
