//! Speaker separation and a word transcript.
//!
//! Diarization uses the Polyvoice balanced profile. Words come from a local
//! Parakeet TDT directory. This crate does not download that directory.

use std::fmt::Display;
use std::path::Path;

use polyvoice::{
    DiarizationResult, ModelRegistry, Pipeline, Profile, SampleRate, SpeakerTurn, who_said_what,
};
use polyvoice_asr::ParakeetAsr;

use crate::audio::Speech;
use crate::error::{Error, Result};

/// One speaker turn.
#[derive(Debug, Clone, PartialEq)]
pub struct SpokenLine {
    /// `SPEAKER_00`, `SPEAKER_01`, and so on.
    pub speaker: String,
    /// Start time in seconds.
    pub start_secs: f64,
    /// End time in seconds.
    pub end_secs: f64,
    /// Words for this turn. Empty when the call did not transcribe.
    pub text: String,
}

/// Separate speakers. The lines have no words.
pub fn separate_speakers(speech: &Speech) -> Result<Vec<SpokenLine>> {
    let pipeline = pipeline()?;
    if speech.samples.is_empty() {
        return Err(Error::EmptyRecording);
    }
    if speech.samples.len() > 16_000 * 3_600 {
        return Err(Error::RecordingTooLong);
    }
    let result = pipeline
        .run(&speech.samples, sample_rate()?)
        .map_err(speakers_err)?;
    Ok(lines(&result.turns))
}

fn pipeline() -> Result<Pipeline> {
    Pipeline::builder()
        .profile(Profile::Balanced)
        .with_models_from(ModelRegistry::default().map_err(speakers_err)?)
        .build()
        .map_err(speakers_err)
}

/// Separate speakers and fill each line with words.
///
/// `model_dir` holds the Parakeet files. Prefer the INT8 names
/// `encoder-model.int8.onnx`, `decoder_joint-model.int8.onnx`, and `vocab.txt`.
///
/// This loads the models for one recording. Use [`TranscriptEngine`] when
/// several recordings share one process.
pub fn transcribe(speech: &Speech, model_dir: &Path) -> Result<Vec<SpokenLine>> {
    TranscriptEngine::open(model_dir)?.transcribe(speech)
}

/// Diarization and speech models kept for more than one recording.
pub struct TranscriptEngine {
    pipeline: Pipeline,
    asr: ParakeetAsr,
}

impl TranscriptEngine {
    /// Load the diarization weights and the Parakeet directory.
    pub fn open(model_dir: &Path) -> Result<Self> {
        let pipeline = pipeline()?;
        let asr = ParakeetAsr::from_dir(model_dir).map_err(transcript_err)?;
        Ok(Self { pipeline, asr })
    }

    /// Separate speakers and fill each line with words.
    pub fn transcribe(&self, speech: &Speech) -> Result<Vec<SpokenLine>> {
        let result = self.diarize(speech)?;
        let spoken = who_said_what(&result.turns, &self.asr, &speech.samples, sample_rate()?)
            .map_err(transcript_err)?;
        Ok(lines(&spoken.turns))
    }

    fn diarize(&self, speech: &Speech) -> Result<DiarizationResult> {
        if speech.samples.is_empty() {
            return Err(Error::EmptyRecording);
        }
        if speech.samples.len() > 16_000 * 3_600 {
            return Err(Error::RecordingTooLong);
        }
        self.pipeline
            .run(&speech.samples, sample_rate()?)
            .map_err(speakers_err)
    }
}

fn lines(turns: &[SpeakerTurn]) -> Vec<SpokenLine> {
    turns
        .iter()
        .map(|turn| SpokenLine {
            speaker: turn.speaker.to_string(),
            start_secs: turn.time.start,
            end_secs: turn.time.end,
            text: turn_text(turn),
        })
        .collect()
}

fn turn_text(turn: &SpeakerTurn) -> String {
    turn.text.clone().unwrap_or_default()
}

fn sample_rate() -> Result<SampleRate> {
    SampleRate::new(16_000).ok_or_else(|| Error::Speakers("16000 Hz was rejected".into()))
}

fn speakers_err(err: impl Display) -> Error {
    Error::Speakers(err.to_string())
}

fn transcript_err(err: impl Display) -> Error {
    Error::Transcript(err.to_string())
}
