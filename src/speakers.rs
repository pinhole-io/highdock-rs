//! Speaker separation and a word transcript.
//!
//! Diarization uses the Polyvoice balanced profile. Words come from a local
//! Parakeet TDT directory. This crate does not download that directory.
//!
//! [`TranscriptEngine`] keeps one speaker id across the recordings it sees.
//! The voice roster stays in memory. This crate does not write it.

use std::collections::VecDeque;
use std::fmt::Display;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::thread;

use polyvoice::utils::l2_normalize;
use polyvoice::{
    Embedder, EmbedderError, ModelRegistry, Pipeline, Profile, ResNet34Native, SampleRate,
    SpeakerTurn, who_said_what,
};
use polyvoice_asr::ParakeetAsr;

use crate::audio::Speech;
use crate::error::{Error, Result};
use crate::roster::{SpeakerRoster, speaker_label, unlinked_label};

/// One speaker turn.
#[derive(Debug, Clone, PartialEq)]
pub struct SpokenLine {
    /// `SPEAKER_00` when the voice is in the roster. `UNLINKED_00` when this
    /// file had too little speech to match.
    pub speaker: String,
    /// Start time in seconds.
    pub start_secs: f64,
    /// End time in seconds.
    pub end_secs: f64,
    /// Words for this turn. Empty when the call did not transcribe.
    pub text: String,
}

/// Slices shorter than this do not enter the roster.
///
/// WeSpeaker ResNet34 needs about 0.20 s. A shorter slice can pad into a
/// vector that does not belong to the voice.
const MIN_EMBED_SECS: f64 = 0.20;

const SAMPLE_RATE_HZ: f64 = 16_000.0;

/// Separate speakers. The lines have no words.
///
/// Ids belong to this file. Use [`TranscriptEngine`] to keep one id across
/// recordings.
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

fn embedder() -> Result<ResNet34Native> {
    let registry = ModelRegistry::default().map_err(speakers_err)?;
    let models = registry
        .ensure_for_profile(Profile::Balanced)
        .map_err(speakers_err)?;
    ResNet34Native::from_onnx_path(&models.embedder_path).map_err(speakers_err)
}

/// Separate speakers and fill each line with words.
///
/// `model_dir` holds the Parakeet files. Prefer the INT8 names
/// `encoder-model.int8.onnx`, `decoder_joint-model.int8.onnx`, and `vocab.txt`.
///
/// This loads the models for one recording. Use [`TranscriptEngine`] when
/// several recordings share one process.
pub fn transcribe(speech: &Speech, model_dir: &Path) -> Result<Vec<SpokenLine>> {
    let mut engine = TranscriptEngine::open(model_dir)?;
    engine.transcribe(speech)
}

/// Diarization, speech models, and the voice roster for more than one recording.
///
/// [`Self::transcribe_batch`] runs several recordings at once. The roster is
/// applied in slice order, so a voice keeps the same id as a one-by-one run.
pub struct TranscriptEngine {
    model_dir: PathBuf,
    pipeline: Pipeline,
    asr: ParakeetAsr,
    /// Extra Parakeet sessions. One session transcribes one file at a time.
    extra_asr: Vec<ParakeetAsr>,
    embedder: ResNet34Native,
    roster: SpeakerRoster,
}

impl TranscriptEngine {
    /// Load the diarization weights, the ResNet34 voice model, and the Parakeet directory.
    pub fn open(model_dir: &Path) -> Result<Self> {
        let pipeline = pipeline()?;
        let embedder = embedder()?;
        let asr = ParakeetAsr::from_dir(model_dir).map_err(transcript_err)?;
        Ok(Self {
            model_dir: model_dir.to_path_buf(),
            pipeline,
            asr,
            extra_asr: Vec::new(),
            embedder,
            roster: SpeakerRoster::new(),
        })
    }

    /// The voice roster for this engine.
    ///
    /// The roster is process memory. This crate does not write it.
    pub fn roster(&self) -> &SpeakerRoster {
        &self.roster
    }

    /// Replace the voice roster.
    ///
    /// Pass a roster from another engine to keep the same ids.
    /// The roster is not written.
    pub fn set_roster(&mut self, roster: SpeakerRoster) {
        self.roster = roster;
    }

    /// Separate speakers and fill each line with words.
    ///
    /// A voice already in the roster keeps its id. A new voice gets the next id.
    /// A speaker with no slice of at least 0.20 s stays `UNLINKED_00` for this
    /// file and does not enter the roster.
    pub fn transcribe(&mut self, speech: &Speech) -> Result<Vec<SpokenLine>> {
        let mut spoken_lines = recognize(&self.pipeline, &self.asr, speech)?;
        link_speakers(
            &self.embedder,
            &mut self.roster,
            &mut spoken_lines,
            &speech.samples,
        )?;
        Ok(spoken_lines)
    }

    /// Transcribe every recording. At most [`transcription_width`] run at once.
    ///
    /// The roster is applied in slice order. One failed recording does not
    /// drop the others. An error from loading a session fails the whole call.
    pub fn transcribe_batch(
        &mut self,
        speeches: &[Speech],
    ) -> Result<Vec<Result<Vec<SpokenLine>>>> {
        if speeches.is_empty() {
            return Ok(Vec::new());
        }
        if speeches.len() == 1 {
            return Ok(vec![self.transcribe(&speeches[0])]);
        }
        let jobs = asr_workers(cpu_count(), speeches.len());
        self.ensure_sessions(jobs)?;
        let _width = FileWidth::new(jobs);
        let slots: Vec<Mutex<Option<Result<Vec<SpokenLine>>>>> =
            (0..speeches.len()).map(|_| Mutex::new(None)).collect();
        let queue = Mutex::new((0..speeches.len()).collect::<VecDeque<_>>());
        let sessions: Vec<&ParakeetAsr> = std::iter::once(&self.asr)
            .chain(self.extra_asr.iter())
            .take(jobs)
            .collect();
        thread::scope(|scope| {
            for asr in sessions {
                let queue = &queue;
                let slots = &slots;
                let pipeline = &self.pipeline;
                scope.spawn(move || {
                    while let Some(index) = pop_index(queue) {
                        let recognized = recognize(pipeline, asr, &speeches[index]);
                        *slots[index].lock().unwrap_or_else(|err| err.into_inner()) =
                            Some(recognized);
                    }
                });
            }
        });
        let mut output = Vec::with_capacity(speeches.len());
        for (speech, slot) in speeches.iter().zip(slots) {
            let recognized = slot
                .into_inner()
                .unwrap_or_else(|err| err.into_inner())
                .unwrap_or_else(|| {
                    Err(Error::Transcript(
                        "the recording was not transcribed".into(),
                    ))
                });
            let linked = match recognized {
                Ok(mut spoken_lines) => link_speakers(
                    &self.embedder,
                    &mut self.roster,
                    &mut spoken_lines,
                    &speech.samples,
                )
                .map(|()| spoken_lines),
                Err(err) => Err(err),
            };
            output.push(linked);
        }
        Ok(output)
    }

    fn ensure_sessions(&mut self, jobs: usize) -> Result<()> {
        while self.extra_asr.len() + 1 < jobs {
            let asr = ParakeetAsr::from_dir(&self.model_dir).map_err(transcript_err)?;
            self.extra_asr.push(asr);
        }
        Ok(())
    }
}

/// How many recordings to transcribe at once.
///
/// Each Parakeet session uses 4 threads. The width stays at or below 4, and
/// at or below one quarter of the CPUs. One file stays on one session.
pub fn transcription_width(files: usize) -> usize {
    asr_workers(cpu_count(), files)
}

fn asr_workers(cpus: usize, files: usize) -> usize {
    if files <= 1 {
        return 1;
    }
    (cpus / 4).clamp(1, 4).min(files)
}

fn cpu_count() -> usize {
    thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
}

fn recognize(pipeline: &Pipeline, asr: &ParakeetAsr, speech: &Speech) -> Result<Vec<SpokenLine>> {
    if speech.samples.is_empty() {
        return Err(Error::EmptyRecording);
    }
    if speech.samples.len() > 16_000 * 3_600 {
        return Err(Error::RecordingTooLong);
    }
    let result = pipeline
        .run(&speech.samples, sample_rate()?)
        .map_err(speakers_err)?;
    let spoken = who_said_what(&result.turns, asr, &speech.samples, sample_rate()?)
        .map_err(transcript_err)?;
    Ok(lines(&spoken.turns))
}

fn pop_index(queue: &Mutex<VecDeque<usize>>) -> Option<usize> {
    queue
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .pop_front()
}

/// Puts the kernel fan-out back to one file when this value drops.
struct FileWidth;

impl FileWidth {
    fn new(jobs: usize) -> Self {
        polyvoice_kernels::set_file_parallelism(jobs);
        Self
    }
}

impl Drop for FileWidth {
    fn drop(&mut self) {
        polyvoice_kernels::set_file_parallelism(1);
    }
}

fn link_speakers(
    embedder: &impl Embedder,
    roster: &mut SpeakerRoster,
    spoken_lines: &mut [SpokenLine],
    samples: &[f32],
) -> Result<()> {
    let order = first_seen(spoken_lines);
    let mut linked = Vec::new();
    let mut vectors = Vec::new();
    let mut unlinked = Vec::new();
    for speaker in order {
        match voice_vector(embedder, spoken_lines, samples, &speaker)? {
            Some(vector) => {
                linked.push(speaker);
                vectors.push(vector);
            }
            None => unlinked.push(speaker),
        }
    }
    let ids = roster.assign(&vectors);
    let mut names = Vec::new();
    for (speaker, id) in linked.into_iter().zip(ids) {
        names.push((speaker, speaker_label(id)));
    }
    for (index, speaker) in unlinked.into_iter().enumerate() {
        names.push((speaker, unlinked_label(index as u32)));
    }
    for line in spoken_lines {
        if let Some((_, label)) = names.iter().find(|(id, _)| id == &line.speaker) {
            line.speaker = label.clone();
        }
    }
    Ok(())
}

fn first_seen(spoken_lines: &[SpokenLine]) -> Vec<String> {
    let mut order = Vec::new();
    for line in spoken_lines {
        if !order.iter().any(|speaker| speaker == &line.speaker) {
            order.push(line.speaker.clone());
        }
    }
    order
}

fn voice_vector(
    embedder: &impl Embedder,
    spoken_lines: &[SpokenLine],
    samples: &[f32],
    speaker: &str,
) -> Result<Option<Vec<f32>>> {
    let mut sum: Option<Vec<f32>> = None;
    let mut count = 0usize;
    for line in spoken_lines.iter().filter(|line| line.speaker == speaker) {
        let Some(slice) = speech_slice(samples, line.start_secs, line.end_secs) else {
            continue;
        };
        let embedded = match embedder.embed(slice) {
            Ok(vector) => vector,
            Err(EmbedderError::AudioTooShort { .. }) => continue,
            Err(err) => return Err(speakers_err(err)),
        };
        if embedded.is_empty() || embedded.iter().any(|value| !value.is_finite()) {
            continue;
        }
        match &mut sum {
            Some(total) if total.len() == embedded.len() => {
                for (slot, value) in total.iter_mut().zip(&embedded) {
                    *slot += value;
                }
                count += 1;
            }
            Some(_) => continue,
            None => {
                sum = Some(embedded);
                count = 1;
            }
        }
    }
    let Some(mut mean) = sum else {
        return Ok(None);
    };
    let scale = count as f32;
    for value in &mut mean {
        *value /= scale;
    }
    l2_normalize(&mut mean);
    if mean.iter().any(|value| !value.is_finite()) || mean.iter().all(|value| *value == 0.0) {
        return Ok(None);
    }
    Ok(Some(mean))
}

fn speech_slice(samples: &[f32], start_secs: f64, end_secs: f64) -> Option<&[f32]> {
    if !start_secs.is_finite() || !end_secs.is_finite() {
        return None;
    }
    let span = end_secs - start_secs;
    if !span.is_finite() || span < MIN_EMBED_SECS {
        return None;
    }
    let start = sample_index(start_secs).min(samples.len());
    let end = sample_index(end_secs).min(samples.len());
    if end <= start {
        return None;
    }
    Some(&samples[start..end])
}

fn sample_index(secs: f64) -> usize {
    if !secs.is_finite() || secs <= 0.0 {
        return 0;
    }
    let samples = secs * SAMPLE_RATE_HZ;
    if samples >= usize::MAX as f64 {
        usize::MAX
    } else {
        samples.round() as usize
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

#[cfg(test)]
mod tests {
    use super::*;

    struct FixedEmbedder;

    impl Embedder for FixedEmbedder {
        fn dim(&self) -> usize {
            2
        }

        fn embed(&self, _audio: &[f32]) -> std::result::Result<Vec<f32>, EmbedderError> {
            Ok(vec![1.0, 0.0])
        }
    }

    fn line(speaker: &str, start: f64, end: f64, text: &str) -> SpokenLine {
        SpokenLine {
            speaker: speaker.to_string(),
            start_secs: start,
            end_secs: end,
            text: text.to_string(),
        }
    }

    #[test]
    fn four_cpus_run_one_file_and_thirty_two_cpus_run_four() {
        assert_eq!(asr_workers(4, 10), 1);
        assert_eq!(asr_workers(8, 10), 2);
        assert_eq!(asr_workers(32, 10), 4);
        assert_eq!(asr_workers(32, 1), 1);
        assert_eq!(asr_workers(32, 3), 3);
    }

    #[test]
    fn the_same_voice_keeps_its_id_on_the_next_recording() {
        let embedder = FixedEmbedder;
        let mut roster = SpeakerRoster::new();
        let samples = vec![0.0; 16_000];
        let mut first = vec![line("SPEAKER_03", 0.0, 1.0, "alpha")];
        link_speakers(&embedder, &mut roster, &mut first, &samples).unwrap();
        let mut second = vec![line("SPEAKER_00", 0.0, 1.0, "beta")];
        link_speakers(&embedder, &mut roster, &mut second, &samples).unwrap();
        assert_eq!(first[0].speaker, "SPEAKER_00");
        assert_eq!(first[0].text, "alpha");
        assert_eq!(second[0].speaker, "SPEAKER_00");
        assert_eq!(second[0].text, "beta");
        assert_eq!(roster.len(), 1);
    }

    #[test]
    fn two_voices_in_one_file_do_not_share_an_id() {
        let embedder = FixedEmbedder;
        let mut roster = SpeakerRoster::new();
        let samples = vec![0.0; 32_000];
        let mut spoken_lines = vec![
            line("LOCAL_A", 0.0, 1.0, "one"),
            line("LOCAL_B", 1.0, 2.0, "two"),
        ];
        link_speakers(&embedder, &mut roster, &mut spoken_lines, &samples).unwrap();
        assert_eq!(spoken_lines[0].speaker, "SPEAKER_00");
        assert_eq!(spoken_lines[1].speaker, "SPEAKER_01");
        assert_eq!(roster.len(), 2);
    }

    #[test]
    fn a_short_turn_stays_unlinked() {
        let embedder = FixedEmbedder;
        let mut roster = SpeakerRoster::new();
        let samples = vec![0.0; 16_000];
        let mut spoken_lines = vec![line("SPEAKER_00", 0.0, 0.19, "nope")];
        link_speakers(&embedder, &mut roster, &mut spoken_lines, &samples).unwrap();
        assert_eq!(spoken_lines[0].speaker, "UNLINKED_00");
        assert!(roster.is_empty());
    }

    #[test]
    fn a_turn_at_the_embed_floor_joins_the_roster() {
        let embedder = FixedEmbedder;
        let mut roster = SpeakerRoster::new();
        let samples = vec![0.0; 16_000];
        let mut spoken_lines = vec![line("SPEAKER_00", 0.0, 0.20, "yes")];
        link_speakers(&embedder, &mut roster, &mut spoken_lines, &samples).unwrap();
        assert_eq!(spoken_lines[0].speaker, "SPEAKER_00");
        assert_eq!(roster.len(), 1);
    }
}
