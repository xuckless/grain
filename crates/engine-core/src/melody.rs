//! Reference note segmentation: pitch contour → discrete notes (AGENTS.md §6.2).

use crate::key::Key;
use crate::pitch::{PitchFrame, PitchTrack};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Median filter length in frames (~50 ms at a 10 ms hop).
const MEDIAN_FRAMES: usize = 5;
/// A note ends at a voicing gap of at least this long.
const MAX_GAP_SECONDS: f64 = 0.040;
/// A new note starts when pitch leaves the running note median by this much…
const SPLIT_CENTS: f32 = 70.0;
/// …for at least this long.
const SPLIT_SECONDS: f64 = 0.060;
/// Notes shorter than this are merged into a neighbor or dropped.
const MIN_NOTE_SECONDS: f64 = 0.080;
/// Extra distance (in semitones) charged to out-of-scale pitches when snapping.
const OUT_OF_SCALE_PENALTY: f32 = 0.25;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Note {
    pub start: f64,
    pub end: f64,
    pub midi: u8,
}

/// A note while it is being collected: frame range plus fractional MIDI values.
struct Segment {
    start: usize,
    end: usize,
    midis: Vec<f32>,
}

impl Segment {
    fn median(&self) -> f32 {
        median(&self.midis)
    }
}

pub fn segment(track: &PitchTrack, key: Key) -> Vec<Note> {
    let hop = track.hop_seconds;
    let midi = smoothed_midi(track);
    let max_gap = (MAX_GAP_SECONDS / hop).round() as usize;
    let split_len = (SPLIT_SECONDS / hop).round().max(1.0) as usize;

    let mut segments: Vec<Segment> = Vec::new();
    let mut current: Option<Segment> = None;
    let mut last_voiced = 0usize;
    for (i, m) in midi.iter().enumerate() {
        let Some(m) = *m else {
            if let Some(seg) = current.take_if(|_| i - last_voiced >= max_gap) {
                segments.push(seg);
            }
            continue;
        };
        last_voiced = i;
        let Some(seg) = current.as_mut() else {
            current = Some(Segment {
                start: i,
                end: i + 1,
                midis: vec![m],
            });
            continue;
        };
        seg.end = i + 1;
        seg.midis.push(m);
        // Split when the last `split_len` frames all sit far from the note's earlier median.
        if seg.midis.len() > 2 * split_len {
            let body = &seg.midis[..seg.midis.len() - split_len];
            let reference = median(body);
            let tail = &seg.midis[seg.midis.len() - split_len..];
            if tail
                .iter()
                .all(|t| (t - reference).abs() * 100.0 > SPLIT_CENTS)
            {
                let tail = tail.to_vec();
                let split_at = seg.end - split_len;
                seg.midis.truncate(seg.midis.len() - split_len);
                seg.end = split_at;
                let done = current.replace(Segment {
                    start: split_at,
                    end: i + 1,
                    midis: tail,
                });
                segments.extend(done);
            }
        }
    }
    segments.extend(current);

    let min_len = (MIN_NOTE_SECONDS / hop).round() as usize;
    let mut notes: Vec<(Segment, u8)> = Vec::new();
    for seg in segments {
        let pitch = snap(seg.median(), key);
        let short = seg.end - seg.start < min_len;
        match notes.last_mut() {
            // Merge short fragments into a touching previous note.
            Some((prev, _)) if short && seg.start <= prev.end + max_gap => {
                prev.end = seg.end;
                prev.midis.extend(seg.midis);
            }
            _ if short => {}
            Some((prev, prev_pitch)) if *prev_pitch == pitch && seg.start <= prev.end + max_gap => {
                prev.end = seg.end;
                prev.midis.extend(seg.midis);
            }
            _ => notes.push((seg, pitch)),
        }
    }
    notes
        .into_iter()
        .map(|(seg, pitch)| Note {
            start: seg.start as f64 * hop,
            end: seg.end as f64 * hop,
            midi: pitch,
        })
        .collect()
}

/// Fractional MIDI per frame after a median filter over voiced neighbors.
fn smoothed_midi(track: &PitchTrack) -> Vec<Option<f32>> {
    let raw: Vec<Option<f32>> = track
        .frames
        .iter()
        .map(|f| match f {
            PitchFrame::Voiced { midi } => Some(*midi),
            PitchFrame::Unvoiced => None,
        })
        .collect();
    let half = MEDIAN_FRAMES / 2;
    (0..raw.len())
        .map(|i| {
            raw[i]?;
            let lo = i.saturating_sub(half);
            let hi = (i + half + 1).min(raw.len());
            let window: Vec<f32> = raw[lo..hi].iter().flatten().copied().collect();
            Some(median(&window))
        })
        .collect()
}

/// Nearest semitone, biased toward the key's scale.
fn snap(midi: f32, key: Key) -> u8 {
    let base = midi.round() as i32;
    (base - 1..=base + 1)
        .map(|c| {
            let penalty = if key.contains(c.rem_euclid(12) as u8) {
                0.0
            } else {
                OUT_OF_SCALE_PENALTY
            };
            (c, (midi - c as f32).abs() + penalty)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(c, _)| c.clamp(0, 127) as u8)
        .unwrap_or(0)
}

fn median(values: &[f32]) -> f32 {
    let mut v = values.to_vec();
    v.sort_by(f32::total_cmp);
    v[v.len() / 2]
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::key::Mode;

    pub const C_MAJOR: Key = Key {
        tonic: 0,
        mode: Mode::Major,
    };

    /// Builds a 10 ms track from (midi or rest, seconds) pairs, with optional vibrato.
    pub fn synthetic_track(parts: &[(Option<f32>, f64)], vibrato_cents: f32) -> PitchTrack {
        let hop = 0.01;
        let mut frames = Vec::new();
        for (midi, seconds) in parts {
            for i in 0..(seconds / hop).round() as usize {
                frames.push(match midi {
                    Some(m) => {
                        let wobble = vibrato_cents / 100.0
                            * (2.0 * std::f32::consts::PI * 5.5 * i as f32 * hop as f32).sin();
                        PitchFrame::Voiced { midi: m + wobble }
                    }
                    None => PitchFrame::Unvoiced,
                });
            }
        }
        PitchTrack {
            hop_seconds: hop,
            frames,
        }
    }

    #[test]
    fn segments_legato_and_detached_notes() {
        let track = synthetic_track(
            &[
                (Some(60.0), 0.5),
                (Some(64.1), 0.4), // legato step: split by pitch change
                (None, 0.2),
                (Some(67.0), 0.3),
                (Some(67.4), 0.05), // short blip: merged
                (None, 0.1),
            ],
            15.0,
        );
        let notes = segment(&track, C_MAJOR);
        let pitches: Vec<u8> = notes.iter().map(|n| n.midi).collect();
        assert_eq!(pitches, vec![60, 64, 67]);
        assert!((notes[0].start - 0.0).abs() < 0.02);
        assert!((notes[1].start - 0.5).abs() < 0.08, "{:?}", notes[1]);
        assert!((notes[2].start - 1.1).abs() < 0.02);
    }

    #[test]
    fn snapping_prefers_scale_tones() {
        assert_eq!(snap(65.55, C_MAJOR), 65); // F, not F#
        assert_eq!(snap(66.2, C_MAJOR), 66); // clearly F# stays F#
    }
}
