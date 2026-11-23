//! Read the firmware version and the recorder serial.

#[path = "common/mod.rs"]
mod common;

#[tokio::main]
async fn main() {
    common::finish(run().await);
}

async fn run() -> highdock_rs::Result<()> {
    let mut device = common::open().await?;
    let info = device.device_info().await?;
    println!(
        "{} {} {} ({})",
        info.model, info.version, info.serial, info.version_number
    );
    Ok(())
}
