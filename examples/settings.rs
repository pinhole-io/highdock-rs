//! Read auto-record, auto-play, and the optional tone flags.

#[path = "common/mod.rs"]
mod common;

#[tokio::main]
async fn main() {
    common::finish(run().await);
}

async fn run() -> highdock_rs::Result<()> {
    let mut device = common::open().await?;
    let settings = device.settings().await?;
    println!("auto_record {}", settings.auto_record);
    println!("auto_play {}", settings.auto_play);
    println!("notification {:?}", settings.notification);
    println!("bluetooth_tone {:?}", settings.bluetooth_tone);
    Ok(())
}
