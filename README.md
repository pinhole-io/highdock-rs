# highdock-rs

A HiDock SDK for people who would like their recordings back, thanks.

HiDock keeps the audio on a little recorder and a cloud. `highdock-rs` skips the cloud. It speaks Jensen over USB, lists the files, pulls them onto your machine, and writes a speaker transcript. Decloud, then pass the mic.

The name is a joke. The transfer is not. This crate does not format the card, factory-reset the recorder, or write firmware. It also does not reset the USB port. A reset during a transfer can wedge the firmware. Put the reset down.

## What you get

The crate is a library. No notes app. No knowledge graph. No account.

It can:

- open the recorder
- read the clock, battery, card, and settings
- list, download, and delete a recording
- decode Opus or MP3 to 16 kHz mono
- split speakers and, with a local speech model, put words on the timeline
- export JSON: one list of people, then the conversation in time order

A 0-byte file is a stem with no flower. The transcribe-all path skips it.

## Bring your own lighter

- Rust 1.94
- a Tokio runtime
- write access to the recorder's usbfs node

On Linux that node is `/dev/bus/usb/BBB/DDD`. A default node is mode `0664` and group `root`, so a normal user cannot open it.

This user is in group `plugdev`. Install the scoped rule, then replug the recorder.

```bash
sudo cp contrib/99-hidock.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules
sudo udevadm trigger --subsystem-match=usb --attr-match=idVendor=10d6
```

The rule covers vendor `10d6` and `3887` for the known product ids. It sets group `plugdev` and mode `0660`.

On Linux, `detach_and_claim_interface` detaches a kernel driver on interface 0 of the matched recorder. One process holds the interface. Do not hotbox the USB bus with a second open.

## Spark it

1. Add the crate.

```toml
highdock-rs = { git = "https://github.com/pinhole-io/highdock-rs" }
```

2. Run on a Tokio runtime.
3. Call `Device::open_first`.
4. Call `device_info`, `list_recordings`, or `download`.

```rust
let mut device = highdock_rs::Device::open_first().await?;
let info = device.device_info().await?;
let recordings = device.list_recordings().await?;
```

Bulk OUT is `0x01`. Bulk IN is `0x82`. Vendor ids are `0x10D6` and `0x3887`.

## When the session goes bad

A protocol fault poisons the session. That one is done.

1. Drop the `Device`.
2. Unplug the recorder.
3. Plug it back in.
4. Open a new `Device`.

Do not send another command on a poisoned session.

A download callback error drains the rest of that file when the recorder keeps sending. The session stays open when the drain finishes.

Call `device_info` before `download` on an H1E. Firmware 6.3 sends an extra empty frame when the last chunk is 8180 bytes. `download` reads the identity itself when the version is not known yet.

## Examples

`cargo test` uses a fake transport. It does not open a recorder and it does not load a speech model.

With a recorder plugged in, one example per call:

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

`delete_recording` removes that file from the card. `set_time` and `set_auto_record` write the clock and the auto-record flag. Read the room before you delete anything.

## Who said that

`decode_recording` accepts Ogg Opus and MPEG-1 Layer III. Both become 16 kHz mono PCM. A file over 256 MiB is rejected. Audio over one hour is rejected. A final Ogg page that ends early keeps the packets before that page.

`separate_speakers` runs the Polyvoice balanced profile. The first call downloads the signed diarization weights.

`transcribe` needs a local Parakeet TDT directory. This crate does not download that directory. Bring your own model. The crate will not go out for it.

`Transcript::from_lines` builds one document.

- `people` lists each speaker, then that speaker's lines in time order
- `conversation` lists every line in the order it started

`TranscriptBook` holds one document per recording. `write_json` writes one file. `write_each` writes one file per recording.

Use the smaller INT8 set from `istupakov/parakeet-tdt-0.6b-v3-onnx`:

- `encoder-model.int8.onnx`
- `decoder_joint-model.int8.onnx`
- `vocab.txt`

The FP32 encoder is `encoder-model.onnx` plus `encoder-model.onnx.data`. That data file is about 2.4 GB. Do not mix it with the INT8 encoder in the same directory. The loader reads `encoder-model.onnx` first. One jar. Do not pack both.

A path that exists is read from disk. Any other argument is an exact file name from `list_recordings`.

```bash
cargo run --example transcribe -- Rec00.hda
cargo run --example transcribe -- Rec00.hda --asr-model /path/to/parakeet-tdt
cargo run --example transcribe -- --all --asr-model /path/to/parakeet-tdt --out ~/hidock-transcripts
```

`--all` writes one `.txt` file and one `.json` file for each recording on the card. It also writes `transcripts.json`, the long one, with every recording. A recording of 0 bytes is skipped.

Already have the text files? Roll them into JSON without touching the recorder:

```bash
cargo run --example transcribe -- --from-lines ~/hidock-transcripts --out ~/hidock-transcripts
```

Without `--asr-model` the lines have times and speaker names. They have no words. You get the seating chart, not the bit.
