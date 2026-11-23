//! Set the auto-record flag.
//!
//! Argument: `on` or `off`. The second settings word stays zero.

#[path = "common/mod.rs"]
mod common;

#[tokio::main]
async fn main() {
    common::finish(run().await);
}

async fn run() -> highdock_rs::Result<()> {
    let enabled = match std::env::args().nth(1).as_deref() {
        Some("on") => true,
        Some("off") => false,
        _ => {
            eprintln!("usage: set_auto_record <on|off>");
            std::process::exit(2);
        }
    };
    let mut device = common::open().await?;
    device.set_auto_record(enabled).await?;
    let settings = device.settings().await?;
    println!("auto_record {}", settings.auto_record);
    println!("auto_play {}", settings.auto_play);
    println!("notification {:?}", settings.notification);
    println!("bluetooth_tone {:?}", settings.bluetooth_tone);
    Ok(())
}
