//! USB claim for one HiDock recorder.
//!
//! Who can reach this: a local process allowed to open the USB device.
//! Worst input: a device that uses a HiDock vendor id and sends a hostile frame.
//! A failure returns the command id and lengths. It does not return recording bytes.
//!
//! On Linux, the claim detaches a kernel driver on interface 0 of that device.

use std::time::Duration;

use nusb::transfer::{Buffer, Bulk, Completion, In, Out, TransferError};
use nusb::{DeviceInfo, Endpoint, Interface};

use crate::error::{Error, Result};
use crate::ids::{
    CLAIM_SETTLE, DeviceModel, EP_IN, EP_OUT, USB_READ_BYTES, is_hidock_vendor, is_known_product,
};
use crate::transport::Transport;

/// A connected recorder that can be opened.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct UsbDeviceId {
    /// USB vendor id.
    pub vendor_id: u16,
    /// USB product id.
    pub product_id: u16,
    /// Recorder family.
    pub model: DeviceModel,
    /// Host-controller id from the USB stack.
    pub bus_id: String,
    /// Device address on that bus.
    pub device_address: u8,
    /// Product string, when the host already has it.
    pub product_name: Option<String>,
}

/// Bulk pipes for one claimed interface.
pub struct UsbTransport {
    _interface: Interface,
    ep_in: Endpoint<Bulk, In>,
    ep_out: Endpoint<Bulk, Out>,
    in_request_len: usize,
}

impl UsbTransport {
    pub(crate) async fn open(id: &UsbDeviceId) -> Result<Self> {
        let info = find_device(id).await?;
        let device = info
            .open()
            .await
            .map_err(|err| open_error(id, &err.to_string()))?;
        // Windows does not support this call. The host has already selected
        // configuration 1 when the claim below succeeds.
        let _ = device.set_configuration(1).await;
        let interface = device
            .detach_and_claim_interface(0)
            .await
            .map_err(Error::usb)?;
        interface.set_alt_setting(0).await.map_err(Error::usb)?;
        tokio::time::sleep(CLAIM_SETTLE).await;

        let ep_out = interface
            .endpoint::<Bulk, Out>(EP_OUT)
            .map_err(Error::usb)?;
        let ep_in = interface.endpoint::<Bulk, In>(EP_IN).map_err(Error::usb)?;
        let max_packet = ep_in.max_packet_size();
        if max_packet == 0 || USB_READ_BYTES < max_packet {
            return Err(Error::usb("usb packet size is outside the read window"));
        }
        let in_request_len = USB_READ_BYTES - (USB_READ_BYTES % max_packet);
        Ok(Self {
            _interface: interface,
            ep_in,
            ep_out,
            in_request_len,
        })
    }
}

impl Transport for UsbTransport {
    async fn write(&mut self, data: &[u8], timeout: Duration) -> Result<()> {
        let mut buffer = Buffer::new(data.len());
        buffer.extend_from_slice(data);
        self.ep_out.submit(buffer);
        let completion = wait_complete(&mut self.ep_out, timeout).await?;
        completion_status(completion)?;
        Ok(())
    }

    async fn read(&mut self, buf: &mut [u8], timeout: Duration) -> Result<usize> {
        let request = self.in_request_len.min(buf.len());
        let max_packet = self.ep_in.max_packet_size();
        let request = request - (request % max_packet.max(1));
        if request == 0 {
            return Err(Error::usb("read buffer is smaller than the usb packet"));
        }
        self.ep_in.submit(Buffer::new(request));
        let completion = wait_complete(&mut self.ep_in, timeout).await?;
        copy_in(completion, buf)
    }
}

/// List connected HiDock recorders. This does not claim an interface.
pub async fn list_usb_devices() -> Result<Vec<UsbDeviceId>> {
    let devices = nusb::list_devices().await.map_err(Error::usb)?;
    Ok(devices.filter(is_hidock_info).map(to_id).collect())
}

async fn find_device(id: &UsbDeviceId) -> Result<DeviceInfo> {
    let mut devices = nusb::list_devices().await.map_err(Error::usb)?;
    devices
        .find(|info| {
            info.vendor_id() == id.vendor_id
                && info.product_id() == id.product_id
                && info.bus_id() == id.bus_id
                && info.device_address() == id.device_address
        })
        .ok_or(Error::NotFound)
}

fn is_hidock_info(info: &DeviceInfo) -> bool {
    if !is_hidock_vendor(info.vendor_id()) {
        return false;
    }
    if is_known_product(info.product_id()) {
        return true;
    }
    let name = info.product_string().unwrap_or("").to_ascii_lowercase();
    name.contains("hidock") || name.contains("jensen")
}

fn open_error(id: &UsbDeviceId, err: &str) -> Error {
    if err.contains("permission denied") {
        return Error::usb(format!(
            "permission denied for /dev/bus/usb/{}/{:03} (vendor {:04x}, product {:04x}). Grant this user write access to that node, then replug the recorder",
            id.bus_id, id.device_address, id.vendor_id, id.product_id
        ));
    }
    Error::usb(err)
}

fn to_id(info: DeviceInfo) -> UsbDeviceId {
    UsbDeviceId {
        vendor_id: info.vendor_id(),
        product_id: info.product_id(),
        model: DeviceModel::from_product_id(info.product_id()),
        bus_id: info.bus_id().to_owned(),
        device_address: info.device_address(),
        product_name: info.product_string().map(str::to_owned),
    }
}

async fn wait_complete<E, D>(endpoint: &mut Endpoint<E, D>, timeout: Duration) -> Result<Completion>
where
    E: nusb::transfer::BulkOrInterrupt,
    D: nusb::transfer::EndpointDirection,
{
    match tokio::time::timeout(timeout, endpoint.next_complete()).await {
        Ok(completion) => Ok(completion),
        Err(_elapsed) => {
            endpoint.cancel_all();
            if endpoint.pending() > 0 {
                let completion = endpoint.next_complete().await;
                if completion.status.is_ok() {
                    return Ok(completion);
                }
            }
            Err(Error::IoTimeout)
        }
    }
}

fn completion_status(completion: Completion) -> Result<()> {
    match completion.status {
        Ok(()) => Ok(()),
        Err(TransferError::Cancelled) => Err(Error::IoTimeout),
        Err(err) => Err(Error::usb(err)),
    }
}

fn copy_in(completion: Completion, buf: &mut [u8]) -> Result<usize> {
    match completion.status {
        Ok(()) => {
            let n = completion.buffer.len();
            if n > buf.len() {
                return Err(Error::usb("usb read exceeded the buffer"));
            }
            buf[..n].copy_from_slice(&completion.buffer[..n]);
            Ok(n)
        }
        Err(TransferError::Cancelled) => Err(Error::IoTimeout),
        Err(err) => Err(Error::usb(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_error_names_the_usbfs_node() {
        let id = UsbDeviceId {
            vendor_id: 0x10d6,
            product_id: 0xb00e,
            model: DeviceModel::P1,
            bus_id: "009".into(),
            device_address: 9,
            product_name: None,
        };
        let Error::Usb { message } = open_error(&id, "permission denied") else {
            panic!("expected a usb error");
        };
        assert!(message.contains("/dev/bus/usb/009/009"));
        assert!(message.contains("10d6"));
        assert!(message.contains("b00e"));
    }
}
