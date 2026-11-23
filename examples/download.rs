//! Copy one recording to a local file.
//!
//! Arguments: recording name, output path.

#[path = "common/mod.rs"]
mod common;

use std::fs;
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    common::finish(run().await);
}

async fn run() -> highdock_rs::Result<()> {
    let mut args = std::env::args().skip(1);
    let Some(name) = args.next() else {
        eprintln!("usage: download <recording-name> <output-path>");
        std::process::exit(2);
    };
    let Some(path) = args.next() else {
        eprintln!("usage: download <recording-name> <output-path>");
        std::process::exit(2);
    };
    let path = PathBuf::from(path);

    let mut device = common::open().await?;
    let recordings = device.list_recordings().await?;
    let Some(recording) = recordings.into_iter().find(|item| item.name == name) else {
        eprintln!("recording is not on the card");
        std::process::exit(2);
    };
    let expected = recording.bytes;
    let mut bytes = Vec::with_capacity(expected as usize);
    device
        .download(&recording, |chunk| {
            bytes.extend_from_slice(chunk);
            Ok(())
        })
        .await?;
    if let Err(err) = fs::write(&path, &bytes) {
        eprintln!("failed to write {}: {err}", path.display());
        std::process::exit(1);
    }
    println!(
        "{} {} bytes -> {}",
        recording.name,
        bytes.len(),
        path.display()
    );
    if bytes.len() as u32 != expected {
        eprintln!(
            "local file is {0} bytes and the catalog says {expected}",
            bytes.len()
        );
        std::process::exit(1);
    }
    Ok(())
}
