//! Delete one recording from the card.
//!
//! Argument: recording name. This removes that file from the recorder.

#[path = "common/mod.rs"]
mod common;

use highdock_rs::{DeleteOutcome, Recording};

#[tokio::main]
async fn main() {
    common::finish(run().await);
}

async fn run() -> highdock_rs::Result<()> {
    let Some(name) = std::env::args().nth(1) else {
        eprintln!("usage: delete_recording <recording-name>");
        std::process::exit(2);
    };
    let recording = Recording::new(name, 0)?;
    let mut device = common::open().await?;
    match device.delete_recording(&recording).await? {
        DeleteOutcome::Deleted => println!("deleted {}", recording.name),
        DeleteOutcome::Missing => println!("missing {}", recording.name),
        other => println!("{other:?}"),
    }
    Ok(())
}
