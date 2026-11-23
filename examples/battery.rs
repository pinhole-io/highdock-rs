//! Read the battery byte and the optional voltage word.

#[path = "common/mod.rs"]
mod common;

#[tokio::main]
async fn main() {
    common::finish(run().await);
}

async fn run() -> highdock_rs::Result<()> {
    let mut device = common::open().await?;
    let battery = device.battery().await?;
    println!("state {:?}", battery.state);
    println!("level {}", battery.level);
    match battery.voltage_raw {
        Some(voltage) => println!("voltage {voltage}"),
        None => println!("voltage absent"),
    }
    Ok(())
}
