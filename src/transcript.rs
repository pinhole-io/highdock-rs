//! A transcript document and its JSON file export.
//!
//! `people` lists each speaker. Each person holds that speaker's lines in
//! time order. `conversation` is those lines in the order they start.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{Error, Result};
use crate::speakers::SpokenLine;

/// One recording as people plus the conversation.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Transcript {
    /// Card file name, for example `2026Nov20-165919-Rec00.hda`.
    pub recording: String,
    /// Speakers in the order of their first line.
    pub people: Vec<Person>,
    /// Every line in start-time order.
    pub conversation: Vec<Line>,
}

/// One speaker and that speaker's lines.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Person {
    /// `SPEAKER_00` for a roster voice. `UNLINKED_00` for a voice this file could not match.
    pub id: String,
    /// This speaker's lines in start-time order.
    pub messages: Vec<TimedText>,
}

/// Words on the timeline, without a speaker id.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TimedText {
    /// Start time in seconds.
    pub start_secs: f64,
    /// End time in seconds.
    pub end_secs: f64,
    /// Length of the line in seconds.
    pub duration_secs: f64,
    /// Words. Empty when the turn has no words.
    pub text: String,
}

/// One line in the conversation.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Line {
    /// Speaker id. The same string as [`Person::id`].
    pub speaker: String,
    /// Start time in seconds.
    pub start_secs: f64,
    /// End time in seconds.
    pub end_secs: f64,
    /// Length of the line in seconds.
    pub duration_secs: f64,
    /// Words. Empty when the turn has no words.
    pub text: String,
}

/// Several recordings in one document.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TranscriptBook {
    /// Recordings in the order they were added.
    pub recordings: Vec<Transcript>,
}

impl Transcript {
    /// Build a document from speaker lines.
    ///
    /// Lines are ordered by start time. A tie keeps the earlier input line.
    pub fn from_lines(recording: impl Into<String>, lines: &[SpokenLine]) -> Self {
        let mut order: Vec<usize> = (0..lines.len()).collect();
        order.sort_by(|&left, &right| {
            lines[left]
                .start_secs
                .total_cmp(&lines[right].start_secs)
                .then_with(|| lines[left].end_secs.total_cmp(&lines[right].end_secs))
                .then(left.cmp(&right))
        });
        let conversation = order
            .into_iter()
            .map(|index| Line::from_spoken(&lines[index]))
            .collect::<Vec<_>>();
        let mut people: Vec<Person> = Vec::new();
        for line in &conversation {
            if let Some(person) = people.iter_mut().find(|person| person.id == line.speaker) {
                person.messages.push(TimedText::from_line(line));
            } else {
                people.push(Person {
                    id: line.speaker.clone(),
                    messages: vec![TimedText::from_line(line)],
                });
            }
        }
        Self {
            recording: recording.into(),
            people,
            conversation,
        }
    }

    /// Pretty JSON for this recording.
    pub fn to_json(&self) -> Result<String> {
        to_json(self)
    }

    /// Write pretty JSON to `path`.
    pub fn write_json(&self, path: &Path) -> Result<()> {
        write_json(self, path)
    }
}

impl TranscriptBook {
    /// An empty book.
    pub fn new() -> Self {
        Self {
            recordings: Vec::new(),
        }
    }

    /// Add one recording. The book keeps insertion order.
    pub fn push(&mut self, transcript: Transcript) {
        self.recordings.push(transcript);
    }

    /// Pretty JSON for every recording.
    pub fn to_json(&self) -> Result<String> {
        to_json(self)
    }

    /// Write one long JSON file.
    pub fn write_json(&self, path: &Path) -> Result<()> {
        write_json(self, path)
    }

    /// Write one JSON file per recording.
    ///
    /// The file name is the recording stem plus `.json`.
    pub fn write_each(&self, dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir).map_err(|err| export_err(dir, &err))?;
        for transcript in &self.recordings {
            let path = dir.join(json_file_name(&transcript.recording));
            transcript.write_json(&path)?;
        }
        Ok(())
    }
}

impl Default for TranscriptBook {
    fn default() -> Self {
        Self::new()
    }
}

impl Line {
    fn from_spoken(line: &SpokenLine) -> Self {
        let start_secs = millis(line.start_secs);
        let end_secs = millis(line.end_secs);
        Self {
            speaker: line.speaker.clone(),
            start_secs,
            end_secs,
            duration_secs: millis(end_secs - start_secs),
            text: line.text.clone(),
        }
    }
}

fn millis(secs: f64) -> f64 {
    (secs * 1_000.0).round() / 1_000.0
}

impl TimedText {
    fn from_line(line: &Line) -> Self {
        Self {
            start_secs: line.start_secs,
            end_secs: line.end_secs,
            duration_secs: line.duration_secs,
            text: line.text.clone(),
        }
    }
}

fn json_file_name(recording: &str) -> PathBuf {
    let stem = Path::new(recording)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("transcript");
    PathBuf::from(format!("{stem}.json"))
}

fn to_json(value: &impl Serialize) -> Result<String> {
    let mut text =
        serde_json::to_string_pretty(value).map_err(|err| export_err(Path::new("json"), &err))?;
    text.push('\n');
    Ok(text)
}

fn write_json(value: &impl Serialize, path: &Path) -> Result<()> {
    let text = serde_json::to_string_pretty(value).map_err(|err| export_err(path, &err))?;
    let mut bytes = text.into_bytes();
    bytes.push(b'\n');
    std::fs::write(path, bytes).map_err(|err| export_err(path, &err))
}

fn export_err(path: &Path, err: &impl std::fmt::Display) -> Error {
    Error::Export {
        path: path.display().to_string(),
        message: err.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<SpokenLine> {
        vec![
            line("SPEAKER_00", 1.10, 3.74, "."),
            line("SPEAKER_00", 0.00, 0.34, "Wait \"there\""),
            line("SPEAKER_01", 3.74, 4.01, ""),
        ]
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
    fn people_follow_first_appearance_and_conversation_follows_time() {
        let transcript = Transcript::from_lines("Rec00.hda", &sample());
        assert_eq!(transcript.people.len(), 2);
        assert_eq!(transcript.people[0].id, "SPEAKER_00");
        assert_eq!(transcript.people[0].messages.len(), 2);
        assert_eq!(transcript.people[0].messages[0].text, "Wait \"there\"");
        assert_eq!(transcript.people[0].messages[0].duration_secs, 0.34);
        assert_eq!(transcript.people[1].id, "SPEAKER_01");
        assert_eq!(transcript.conversation[0].start_secs, 0.00);
        assert_eq!(transcript.conversation[2].speaker, "SPEAKER_01");
        let json = transcript.to_json().unwrap();
        assert!(json.contains("\"people\""));
        assert!(json.contains("\"conversation\""));
        assert!(json.contains("Wait \\\"there\\\""));
    }

    #[test]
    fn a_book_writes_one_long_file_and_one_file_per_recording() {
        let dir = std::env::temp_dir().join(format!("hidock-transcript-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut book = TranscriptBook::new();
        book.push(Transcript::from_lines("../Rec00.hda", &sample()));
        book.push(Transcript::from_lines("Wip08.hda", &[]));
        book.write_each(&dir).unwrap();
        book.write_json(&dir.join("transcripts.json")).unwrap();
        let one = std::fs::read_to_string(dir.join("Rec00.json")).unwrap();
        assert!(one.contains("SPEAKER_00"));
        let empty = std::fs::read_to_string(dir.join("Wip08.json")).unwrap();
        assert!(empty.contains("\"people\": []"));
        let long = std::fs::read_to_string(dir.join("transcripts.json")).unwrap();
        assert!(long.contains("\"recordings\""));
        assert!(long.contains("Wip08.hda"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
