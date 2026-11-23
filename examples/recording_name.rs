//! Read the name of the file the recorder is writing now.

#[path = "common/mod.rs"]
mod common;

#[tokio::main]
async fn main() {
    common::finish(run().await);
}

async fn run() -> highdock_rs::Result<()> {
    let mut device = common::open().await?;
    match device.recording_name().await? {
        Some(name) => println!("recording {name}"),
        None => println!("idle"),
    }
    Ok(())
}
