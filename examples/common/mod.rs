//! Shared open and error path for the device examples.

use highdock_rs::{Device, UsbTransport};

pub async fn open() -> highdock_rs::Result<Device<UsbTransport>> {
    Device::open_first().await
}

pub fn finish(result: highdock_rs::Result<()>) {
    if let Err(err) = result {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
