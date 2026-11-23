# highdock-rs

`highdock-rs` is a Rust SDK for a HiDock recorder.

The SDK speaks the Jensen protocol over USB. It lists recordings, downloads them, and reads device status.

## Scope

The crate is a library.

It opens the recorder, downloads a recording, and turns that file into speaker lines. It does not store notes.

The crate does not format the card, reset the recorder, or write firmware. A USB port reset during a transfer can wedge the recorder. This crate does not reset the port.

## Requirements

- Rust 1.94
- A Tokio runtime
- Write access to the recorder's usbfs node

On Linux the node is `/dev/bus/usb/BBB/DDD`. A default node is mode `0664` and group `root`, so a normal user cannot open it.

This user is in group `plugdev`. Install the scoped rule, then replug the recorder.

```bash
sudo cp contrib/99-hidock.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=usb --attr-match=idVendor=10d6
```

The rule covers vendor `10d6` and `3887` for the known product ids. It sets group `plugdev` and mode `0660`.

On Linux, `detach_and_claim_interface` detaches a kernel driver on interface 0 of the matched recorder.

## Open a recorder

1. Add the `highdock-rs` crate to your package.

```toml
highdock-rs = { git = "https://github.com/pinhole-io/highdock-rs" }
```

2. Run the code on a Tokio runtime.
3. Call `Device::open_first`.
4. Call `device_info`, `list_recordings`, or `download`.

```rust
let mut device = highdock_rs::Device::open_first().await?;
let info = device.device_info().await?;
let recordings = device.list_recordings().await?;
```

The host uses bulk OUT `0x01` and bulk IN `0x82`. Vendor ids are `0x10D6` and `0x3887`.

## Failure

A protocol fault marks the session poisoned.

1. Drop the `Device`.
2. Unplug the recorder.
3. Plug the recorder in again.
4. Open a new `Device`.

Do not send another command on a poisoned session.

A download callback error drains the rest of that file when the recorder keeps sending. The session stays open when the drain finishes.

Call `device_info` before `download` on an H1E. Firmware 6.3 sends an extra empty frame when the last chunk is 8180 bytes. `download` reads the identity itself when the version is not known yet.

## Test

Run `cargo test`.

The tests use a fake transport. They do not open a recorder.

Run one example when a recorder is connected:

- `device_info`
- `device_time`
- `set_time`
- `settings`
- `set_auto_record`
- `card`
- `battery`
- `recording_name`
- `list_recordings`
- `download`
- `delete_recording`
- `transcribe`

`delete_recording` removes the named file from the card. `set_time` and `set_auto_record` write the clock and the auto-record flag.

## Transcript

`decode_recording` accepts Ogg Opus and MPEG-1 Layer III. Both become 16 kHz mono PCM. A file over 256 MiB is rejected. Audio over one hour is rejected.

`separate_speakers` runs the Polyvoice balanced profile. The first call downloads the signed diarization weights.

`transcribe` needs a local Parakeet TDT directory. This crate does not download it.

`Transcript::from_lines` builds one document. `people` lists each speaker and that speaker's lines in time order. `conversation` lists every line in start-time order. `TranscriptBook` holds one document per recording. `write_json` writes the document. `write_each` writes one file per recording.

Use the smaller INT8 set from `istupakov/parakeet-tdt-0.6b-v3-onnx`:

- `encoder-model.int8.onnx`
- `decoder_joint-model.int8.onnx`
- `vocab.txt`

The FP32 encoder is `encoder-model.onnx` plus `encoder-model.onnx.data`. That data file is about 2.4 GB. Do not mix it with the INT8 encoder in the same directory. The loader reads `encoder-model.onnx` first.

A path that exists is read from disk. Any other argument is an exact file name from `list_recordings`.

```bash
cargo run --example transcribe -- 2026Nov20-165919-Rec00.hda
cargo run --example transcribe -- 2026Nov20-165919-Rec00.hda --asr-model /path/to/parakeet-tdt
cargo run --example transcribe -- --all --asr-model /path/to/parakeet-tdt --out ~/hidock-transcripts
```

`--all` writes one `.txt` file and one `.json` file for each recording on the card. It also writes `transcripts.json` for every recording. A recording of 0 bytes is skipped.

```bash
cargo run --example transcribe -- --from-lines ~/hidock-transcripts --out ~/hidock-transcripts
```

Without `--asr-model` the lines have times and speaker names. They have no words.
