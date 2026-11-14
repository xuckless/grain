//! MIDI layers (AGENTS.md §4.3): note transcriptions of a song's tracks, kept with the project,
//! drawn as lanes under the stems and exported as Standard MIDI Files.
//!
//! Pitched tracks are transcribed by basic-pitch (`engine-ml::basic_pitch`, inference in the
//! host); the drum stem by the onset classifier in [`crate::drums`].

use crate::project::{Project, Stem, TrackSource};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// One transcribed note (song seconds).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct MidiNote {
    pub start: f64,
    pub end: f64,
    /// MIDI key (General MIDI drum keys for drums: 36 kick, 38 snare, 42 closed hat).
    pub pitch: u8,
    pub velocity: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum TranscriptionMethod {
    /// Spotify's basic-pitch (polyphonic, any pitched instrument).
    BasicPitch,
    /// Onsets classified into kick, snare and hi-hat.
    DrumClassifier,
}

/// A track's MIDI layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Transcription {
    pub source: TrackSource,
    pub method: TranscriptionMethod,
    pub notes: Vec<MidiNote>,
}

/// How a track is transcribed: the drum stem by the classifier, anything else by basic-pitch.
pub fn method_for(source: TrackSource) -> TranscriptionMethod {
    match source {
        TrackSource::Stem { stem: Stem::Drums } => TranscriptionMethod::DrumClassifier,
        TrackSource::Stem { .. } | TrackSource::Mix => TranscriptionMethod::BasicPitch,
    }
}

/// Keeps `t` with the project, replacing that track's previous layer.
pub fn store(project: &mut Project, t: Transcription) {
    match project
        .transcriptions
        .iter_mut()
        .find(|x| x.source == t.source)
    {
        Some(x) => *x = t,
        None => project.transcriptions.push(t),
    }
}

pub fn transcription(project: &Project, source: TrackSource) -> Option<&Transcription> {
    project.transcriptions.iter().find(|t| t.source == source)
}

/// Ticks per quarter note in exported files.
const PPQ: u16 = 480;
/// General MIDI's drum channel (10, zero-based 9).
const DRUM_CHANNEL: u8 = 9;

/// A type-0 Standard MIDI File of `t` at `bpm` (drums on the GM drum channel).
pub fn smf(t: &Transcription, bpm: f64) -> Vec<u8> {
    let bpm = if bpm.is_finite() && bpm > 0.0 {
        bpm
    } else {
        120.0
    };
    let channel = match t.method {
        TranscriptionMethod::DrumClassifier => DRUM_CHANNEL,
        TranscriptionMethod::BasicPitch => 0,
    };
    let ticks = |s: f64| (s.max(0.0) * bpm / 60.0 * f64::from(PPQ)).round() as u32;
    // (tick, order, bytes): note-offs sort before note-ons at the same tick.
    let mut events: Vec<(u32, u8, [u8; 3])> = Vec::with_capacity(t.notes.len() * 2);
    for n in &t.notes {
        let (on, off) = (ticks(n.start), ticks(n.end).max(ticks(n.start) + 1));
        let key = n.pitch.min(127);
        events.push((on, 1, [0x90 | channel, key, n.velocity.clamp(1, 127)]));
        events.push((off, 0, [0x80 | channel, key, 0]));
    }
    events.sort_by_key(|e| (e.0, e.1));

    let mut track = Vec::new();
    // Tempo meta event.
    let tempo = (60_000_000.0 / bpm).round() as u32;
    track.extend([0x00, 0xFF, 0x51, 0x03]);
    track.extend(&tempo.to_be_bytes()[1..]);
    let mut last = 0u32;
    for (tick, _, bytes) in events {
        vlq(&mut track, tick - last);
        track.extend(bytes);
        last = tick;
    }
    track.extend([0x00, 0xFF, 0x2F, 0x00]);

    let mut out = Vec::with_capacity(track.len() + 22);
    out.extend(b"MThd");
    out.extend(6u32.to_be_bytes());
    out.extend(0u16.to_be_bytes()); // format 0
    out.extend(1u16.to_be_bytes()); // one track
    out.extend(PPQ.to_be_bytes());
    out.extend(b"MTrk");
    out.extend((track.len() as u32).to_be_bytes());
    out.extend(track);
    out
}

/// MIDI variable-length quantity.
fn vlq(out: &mut Vec<u8>, mut v: u32) {
    let mut bytes = [0u8; 5];
    let mut n = 0;
    loop {
        bytes[n] = (v & 0x7F) as u8;
        n += 1;
        v >>= 7;
        if v == 0 {
            break;
        }
    }
    for i in (0..n).rev() {
        out.push(bytes[i] | if i > 0 { 0x80 } else { 0 });
    }
}

/// File name for a track's export, e.g. "Bass.mid".
pub fn file_name(source: TrackSource) -> String {
    match source {
        TrackSource::Mix => "Mix.mid".to_string(),
        TrackSource::Stem { stem } => format!("{stem:?}.mid"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn vlq_matches_the_spec_examples() {
        for (v, want) in [
            (0u32, vec![0x00]),
            (0x40, vec![0x40]),
            (0x7F, vec![0x7F]),
            (0x80, vec![0x81, 0x00]),
            (0x2000, vec![0xC0, 0x00]),
            (0x0FFF_FFFF, vec![0xFF, 0xFF, 0xFF, 0x7F]),
        ] {
            let mut out = Vec::new();
            vlq(&mut out, v);
            assert_eq!(out, want, "{v:#x}");
        }
    }

    #[test]
    fn smf_has_header_tempo_and_paired_notes() {
        let t = Transcription {
            source: TrackSource::Stem { stem: Stem::Drums },
            method: TranscriptionMethod::DrumClassifier,
            notes: vec![
                MidiNote {
                    start: 0.0,
                    end: 0.1,
                    pitch: 36,
                    velocity: 100,
                },
                MidiNote {
                    start: 0.5,
                    end: 0.6,
                    pitch: 38,
                    velocity: 90,
                },
            ],
        };
        let f = smf(&t, 120.0);
        assert_eq!(&f[..4], b"MThd");
        assert_eq!(&f[8..14], &[0, 0, 0, 1, 0x01, 0xE0]);
        assert_eq!(&f[14..18], b"MTrk");
        let len = u32::from_be_bytes(f[18..22].try_into().unwrap()) as usize;
        assert_eq!(f.len(), 22 + len);
        // 500 000 µs per quarter at 120 bpm.
        assert_eq!(&f[22..29], &[0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20]);
        // Drums on channel 10.
        assert_eq!(f.iter().filter(|&&b| b == 0x99).count(), 2);
        assert_eq!(f.iter().filter(|&&b| b == 0x89).count(), 2);
        assert_eq!(&f[f.len() - 3..], &[0xFF, 0x2F, 0x00]);
    }

    #[test]
    fn drums_use_the_classifier() {
        assert_eq!(
            method_for(TrackSource::Stem { stem: Stem::Drums }),
            TranscriptionMethod::DrumClassifier
        );
        assert_eq!(
            method_for(TrackSource::Stem { stem: Stem::Bass }),
            TranscriptionMethod::BasicPitch
        );
        assert_eq!(
            method_for(TrackSource::Mix),
            TranscriptionMethod::BasicPitch
        );
    }
}
