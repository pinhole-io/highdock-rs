//! Read card free space and capacity.

#[path = "common/mod.rs"]
mod common;

#[tokio::main]
async fn main() {
    common::finish(run().await);
}

async fn run() -> highdock_rs::Result<()> {
    let mut device = common::open().await?;
    let card = device.card().await?;
    println!("free {} MiB", card.free_mib);
    println!("capacity {} MiB", card.capacity_mib);
    println!("used {} MiB", card.used_mib());
    println!("status {:#x}", card.status_raw);
    Ok(())
}
