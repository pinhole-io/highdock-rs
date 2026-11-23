//! Decode a HiDock recording to 16 kHz mono PCM.
//!
//! A P1 file starts as Ogg Opus or as MPEG-1 Layer III. WAV is accepted too.

use std::io::{Cursor, Read, Seek, SeekFrom};

use opus_pure::{MAX_PACKET_SAMPLES, OggOpusReader, Trim};
use rubato::{
    Resampler, SincFixedIn, SincInterpolationParameters, SincInterpolationType, WindowFunction,
};
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{CODEC_TYPE_NULL, DecoderOptions};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

use crate::error::{Error, Result};

/// One hour of 16 kHz mono audio.
const MAX_SAMPLES: usize = 16_000 * 3_600;

/// Largest recording this crate will read.
const MAX_INPUT_BYTES: u64 = 256 * 1024 * 1024;

const TARGET_RATE: u32 = 16_000;

/// 16 kHz mono PCM in the range `-1.0` to `1.0`.
#[derive(Debug, Clone, PartialEq)]
pub struct Speech {
    /// Samples at 16 kHz.
    pub samples: Vec<f32>,
}

/// Decode one recording.
///
/// Opus is decoded at 16 kHz. MP3 and WAV are resampled to 16 kHz.
/// A final Ogg page that ends early keeps the packets before that page.
pub fn decode_recording(bytes: &[u8]) -> Result<Speech> {
    if bytes.is_empty() {
        return Err(Error::EmptyRecording);
    }
    let len = bytes.len() as u64;
    if len > MAX_INPUT_BYTES {
        return Err(Error::RecordingTooLarge {
            bytes: len,
            limit: MAX_INPUT_BYTES,
        });
    }
    let samples = match container(bytes)? {
        Container::Opus => decode_opus(bytes)?,
        Container::Mp3 => {
            let (pcm, rate) = decode_symphonia(bytes, "mp3")?;
            to_16k(pcm, rate)?
        }
        Container::Wav => {
            let (pcm, rate) = decode_symphonia(bytes, "wav")?;
            to_16k(pcm, rate)?
        }
    };
    if samples.is_empty() {
        return Err(Error::Decode("the recording has no samples".into()));
    }
    if samples.len() > MAX_SAMPLES {
        return Err(Error::RecordingTooLong);
    }
    Ok(Speech { samples })
}

enum Container {
    Opus,
    Mp3,
    Wav,
}

fn container(bytes: &[u8]) -> Result<Container> {
    if bytes.starts_with(b"OggS") {
        return Ok(Container::Opus);
    }
    if bytes.starts_with(b"ID3") || mp3_frame(bytes) {
        return Ok(Container::Mp3);
    }
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE") {
        return Ok(Container::Wav);
    }
    Err(Error::AudioFormat)
}

fn mp3_frame(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes[0] == 0xff && bytes[1] & 0xe0 == 0xe0
}

fn decode_opus(bytes: &[u8]) -> Result<Vec<f32>> {
    let mut reader = OggOpusReader::new(Cursor::new(bytes)).map_err(decode_err)?;
    let head = reader.head().clone();
    let channels = usize::from(head.channel_count);
    if channels == 0 || channels > 2 {
        return Err(Error::Decode("Opus channel count is not 1 or 2".into()));
    }
    let mut decoder = head.decoder(TARGET_RATE as i32).map_err(decode_err)?;
    let mut trim = Trim::new(&head, TARGET_RATE as i32, channels).map_err(decode_err)?;
    let mut block = vec![0.0f32; MAX_PACKET_SAMPLES * channels];
    let mut samples = Vec::new();
    for packet in reader.packets() {
        let packet = match packet {
            Ok(packet) => packet,
            Err(err) if !samples.is_empty() && ogg_cut_off(&err) => break,
            Err(err) => return Err(decode_err(err)),
        };
        let frames = decoder
            .decode(&packet.data, MAX_PACKET_SAMPLES, &mut block)
            .map_err(decode_err)?;
        let kept = trim.keep(&packet, &block[..frames * channels]);
        append_mono(&mut samples, kept, channels)?;
    }
    Ok(samples)
}

fn ogg_cut_off(err: &opus_pure::Error) -> bool {
    match err {
        opus_pure::Error::InvalidStream(message) => matches!(
            *message,
            "stream ends inside a page"
                | "stream ends inside a page header"
                | "stream ends without a valid page"
                | "stream ends in the middle of a packet"
        ),
        _ => false,
    }
}

fn append_mono(samples: &mut Vec<f32>, interleaved: &[f32], channels: usize) -> Result<()> {
    if channels == 1 {
        samples.extend_from_slice(interleaved);
    } else {
        for frame in interleaved.chunks_exact(channels) {
            let sum: f32 = frame.iter().copied().sum();
            samples.push(sum / channels as f32);
        }
    }
    if samples.len() > MAX_SAMPLES + TARGET_RATE as usize {
        return Err(Error::RecordingTooLong);
    }
    Ok(())
}

fn decode_symphonia(bytes: &[u8], extension: &str) -> Result<(Vec<f32>, u32)> {
    let source = OwnedBytes {
        cursor: Cursor::new(bytes.to_vec()),
    };
    let stream = MediaSourceStream::new(Box::new(source), Default::default());
    let mut hint = Hint::new();
    hint.with_extension(extension);
    let probed = symphonia::default::get_probe()
        .format(
            &hint,
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(decode_err)?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|track| track.codec_params.codec != CODEC_TYPE_NULL)
        .cloned()
        .ok_or_else(|| Error::Decode("the file has no audio track".into()))?;
    let rate = track
        .codec_params
        .sample_rate
        .ok_or_else(|| Error::Decode("the codec did not report a sample rate".into()))?;
    if rate == 0 {
        return Err(Error::Decode("the sample rate is 0".into()));
    }
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(decode_err)?;
    let track_id = track.id;
    let mut mono = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(SymphoniaError::ResetRequired) | Err(SymphoniaError::IoError(_)) => break,
            Err(err) => return Err(decode_err(err)),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(decoded) => decoded,
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(err) => return Err(decode_err(err)),
        };
        let spec = *decoded.spec();
        let mut buffer = SampleBuffer::<f32>::new(decoded.capacity() as u64, spec);
        buffer.copy_interleaved_ref(decoded);
        let channels = spec.channels.count().max(1);
        if channels == 1 {
            mono.extend_from_slice(buffer.samples());
        } else {
            for frame in buffer.samples().chunks(channels) {
                let sum: f32 = frame.iter().copied().sum();
                mono.push(sum / channels as f32);
            }
        }
        if mono.len() as f64 / f64::from(rate) > 3_600.0 {
            return Err(Error::RecordingTooLong);
        }
    }
    Ok((mono, rate))
}

fn to_16k(samples: Vec<f32>, rate: u32) -> Result<Vec<f32>> {
    if rate == TARGET_RATE {
        return Ok(samples);
    }
    if samples.is_empty() {
        return Ok(samples);
    }
    let chunk = 1024.min(samples.len());
    let params = SincInterpolationParameters {
        sinc_len: 256,
        f_cutoff: 0.95,
        interpolation: SincInterpolationType::Linear,
        oversampling_factor: 256,
        window: WindowFunction::BlackmanHarris2,
    };
    let mut resampler = SincFixedIn::<f32>::new(
        f64::from(TARGET_RATE) / f64::from(rate),
        2.0,
        params,
        chunk,
        1,
    )
    .map_err(decode_err)?;
    let mut output = Vec::new();
    let mut offset = 0;
    while offset + chunk <= samples.len() {
        let frames = resampler
            .process(&[&samples[offset..offset + chunk]], None)
            .map_err(decode_err)?;
        output.extend_from_slice(&frames[0]);
        offset += chunk;
    }
    let tail = if offset < samples.len() {
        Some(&samples[offset..])
    } else {
        None
    };
    let frames = match tail {
        Some(rest) => resampler.process_partial(Some(&[rest]), None),
        None => resampler.process_partial::<&[f32]>(None, None),
    }
    .map_err(decode_err)?;
    output.extend_from_slice(&frames[0]);
    Ok(output)
}

fn decode_err(err: impl std::fmt::Display) -> Error {
    Error::Decode(err.to_string())
}

struct OwnedBytes {
    cursor: Cursor<Vec<u8>>,
}

impl Read for OwnedBytes {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.cursor.read(buf)
    }
}

impl Seek for OwnedBytes {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        self.cursor.seek(pos)
    }
}

impl MediaSource for OwnedBytes {
    fn is_seekable(&self) -> bool {
        true
    }

    fn byte_len(&self) -> Option<u64> {
        Some(self.cursor.get_ref().len() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use opus_pure::{Application, MAX_PACKET_BYTES, OggOpusWriter, OpusEncoder, OpusHead};

    #[test]
    fn empty_and_unknown_bytes_fail() {
        assert!(matches!(decode_recording(&[]), Err(Error::EmptyRecording)));
        assert!(matches!(
            decode_recording(b"not-audio"),
            Err(Error::AudioFormat)
        ));
    }

    #[test]
    fn a_generated_opus_page_decodes() {
        let bytes = sine_ogg();
        let speech = decode_recording(&bytes).unwrap();
        assert!(speech.samples.len() > 8_000);
        let energy: f32 = speech.samples.iter().map(|sample| sample.abs()).sum();
        assert!(energy / speech.samples.len() as f32 > 0.01);
    }

    #[test]
    fn a_cut_off_opus_page_keeps_earlier_audio() {
        let bytes = sine_ogg();
        let full = decode_recording(&bytes).unwrap();
        let mut cut = bytes;
        cut.truncate(cut.len().saturating_sub(48));
        let speech = decode_recording(&cut).unwrap();
        assert!(speech.samples.len() > 4_000);
        assert!(speech.samples.len() < full.samples.len());
    }

    fn sine_ogg() -> Vec<u8> {
        let rate = 48_000usize;
        let mut pcm = Vec::with_capacity(rate);
        for index in 0..rate {
            let t = index as f32 / rate as f32;
            pcm.push((2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.2);
        }
        let mut encoder = OpusEncoder::new(rate as i32, 1, Application::Audio).unwrap();
        let head = OpusHead::for_encoder(&encoder, rate as u32);
        let mut writer = OggOpusWriter::new(Vec::new(), head).unwrap();
        let mut packet = vec![0u8; MAX_PACKET_BYTES];
        for frame in pcm.chunks_exact(960) {
            let len = encoder.encode(frame, 960, &mut packet).unwrap();
            writer.write_packet(&packet[..len]).unwrap();
        }
        writer.finish().unwrap()
    }
}
