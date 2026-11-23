//! Read the recorder clock.

#[path = "common/mod.rs"]
mod common;

use highdock_rs::DeviceClock;

#[tokio::main]
async fn main() {
    common::finish(run().await);
}

async fn run() -> highdock_rs::Result<()> {
    let mut device = common::open().await?;
    match device.device_time().await? {
        DeviceClock::Unknown => println!("clock unknown"),
        DeviceClock::Set(time) => println!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            time.year(),
            time.month(),
            time.day(),
            time.hour(),
            time.minute(),
            time.second()
        ),
        other => println!("clock {other:?}"),
    }
    Ok(())
}
