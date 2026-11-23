//! List the recordings stored on the card.

#[path = "common/mod.rs"]
mod common;

#[tokio::main]
async fn main() {
    common::finish(run().await);
}

async fn run() -> highdock_rs::Result<()> {
    let mut device = common::open().await?;
    let recordings = device.list_recordings().await?;
    println!("{} recordings", recordings.len());
    for recording in &recordings {
        println!("{} {} bytes", recording.name, recording.bytes);
    }
    Ok(())
}
