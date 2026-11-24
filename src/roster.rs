//! In-memory voice roster.
//!
//! One recording clusters speakers on its own. This roster keeps an id when
//! the same voice appears in a later recording. The crate does not write it.

use polyvoice::utils::{cosine_similarity, l2_normalize};

/// Cosine similarity that reuses a roster id.
///
/// This is the Polyvoice balanced-profile clustering threshold.
const MATCH_COSINE: f32 = 0.45;

/// One voice held for later recordings.
#[derive(Debug, Clone)]
struct Voice {
    id: u32,
    vector: Vec<f32>,
    count: u32,
}

/// Voices matched across recordings.
///
/// The roster stays in process memory. This crate does not write it.
/// Transcript JSON does not include the voice vector.
#[derive(Debug, Clone, Default)]
pub struct SpeakerRoster {
    voices: Vec<Voice>,
}

impl SpeakerRoster {
    /// An empty roster.
    pub fn new() -> Self {
        Self { voices: Vec::new() }
    }

    /// The number of voices stored.
    pub fn len(&self) -> usize {
        self.voices.len()
    }

    /// `true` when no voice is stored.
    pub fn is_empty(&self) -> bool {
        self.voices.is_empty()
    }

    /// Give each local voice a stable id.
    ///
    /// `locals` holds one unit vector per speaker, in first-seen order.
    /// A match keeps the roster id. Two locals cannot take the same id.
    /// A local with no match mints the next id.
    pub(crate) fn assign(&mut self, locals: &[Vec<f32>]) -> Vec<u32> {
        let locals: Vec<Vec<f32>> = locals.iter().map(|vector| unit(vector)).collect();
        let mut pairs = Vec::new();
        for (local_index, local) in locals.iter().enumerate() {
            for (roster_index, voice) in self.voices.iter().enumerate() {
                let score = cosine_similarity(local, &voice.vector);
                if score.is_finite() && score >= MATCH_COSINE {
                    pairs.push((score, local_index, roster_index));
                }
            }
        }
        pairs.sort_by(|left, right| {
            right
                .0
                .total_cmp(&left.0)
                .then(left.1.cmp(&right.1))
                .then(left.2.cmp(&right.2))
        });
        let mut local_taken = vec![false; locals.len()];
        let mut roster_taken = vec![false; self.voices.len()];
        let mut chosen = vec![None; locals.len()];
        for (_, local_index, roster_index) in pairs {
            if local_taken[local_index] || roster_taken[roster_index] {
                continue;
            }
            local_taken[local_index] = true;
            roster_taken[roster_index] = true;
            chosen[local_index] = Some(roster_index);
        }
        let mut ids = Vec::with_capacity(locals.len());
        for (local_index, local) in locals.iter().enumerate() {
            if let Some(roster_index) = chosen[local_index] {
                let id = self.voices[roster_index].id;
                absorb(&mut self.voices[roster_index], local);
                ids.push(id);
            } else {
                let id = self.next_id();
                self.voices.push(Voice {
                    id,
                    vector: local.clone(),
                    count: 1,
                });
                ids.push(id);
            }
        }
        ids
    }

    fn next_id(&self) -> u32 {
        self.voices
            .iter()
            .map(|voice| voice.id)
            .max()
            .map(|id| id.saturating_add(1))
            .unwrap_or(0)
    }
}

/// `SPEAKER_00`, `SPEAKER_01`, and so on.
pub(crate) fn speaker_label(id: u32) -> String {
    format!("SPEAKER_{id:02}")
}

/// A voice this file could not match. The label dies with the file.
pub(crate) fn unlinked_label(index: u32) -> String {
    format!("UNLINKED_{index:02}")
}

fn unit(vector: &[f32]) -> Vec<f32> {
    let mut copy = vector.to_vec();
    l2_normalize(&mut copy);
    copy
}

fn absorb(voice: &mut Voice, incoming: &[f32]) {
    if voice.count == u32::MAX || voice.vector.len() != incoming.len() {
        return;
    }
    let count = voice.count as f32;
    let next = count + 1.0;
    for (slot, &sample) in voice.vector.iter_mut().zip(incoming.iter()) {
        *slot = (*slot * count + sample) / next;
    }
    l2_normalize(&mut voice.vector);
    voice.count = voice.count.saturating_add(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_vector_keeps_the_same_id() {
        let mut roster = SpeakerRoster::new();
        let first = roster.assign(&[vec![1.0, 0.0]]);
        let second = roster.assign(&[vec![1.0, 0.0]]);
        assert_eq!(first, vec![0]);
        assert_eq!(second, vec![0]);
        assert_eq!(roster.len(), 1);
    }

    #[test]
    fn new_vector_mints_the_next_id() {
        let mut roster = SpeakerRoster::new();
        assert_eq!(roster.assign(&[vec![1.0, 0.0]]), vec![0]);
        assert_eq!(roster.assign(&[vec![0.0, 1.0]]), vec![1]);
        assert_eq!(roster.len(), 2);
    }

    #[test]
    fn two_locals_cannot_share_one_roster_id() {
        let mut roster = SpeakerRoster::new();
        roster.assign(&[vec![1.0, 0.0]]);
        let ids = roster.assign(&[vec![1.0, 0.0], vec![1.0, 0.0]]);
        assert_eq!(ids, vec![0, 1]);
        assert_eq!(roster.len(), 2);
    }
}
