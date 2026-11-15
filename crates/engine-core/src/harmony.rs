//! Rule-based harmony (AGENTS.md §8, Produce → Harmony): diatonic thirds and sixths above or
//! below a melody in the song's key, rendered from the singer's own voice by the formant-
//! preserving shifter ([`crate::shift`]), so a harmony sounds like the same singer. The melody is
//! the vocal stem's reference notes or a take's segmented notes; [`melody_midi`] exports either
//! (or its harmony) as a Standard MIDI File.

use crate::edit::{self, EditError, EditSource};
use crate::key::Key;
use crate::melody::{self, Note};
use crate::midi::{self, MidiNote};
use crate::pitch::{PitchFrame, PitchTrack};
use crate::project::Project;
use crate::shift;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Interval {
    ThirdAbove,
    ThirdBelow,
    SixthAbove,
    SixthBelow,
}

impl Interval {
    pub const ALL: [Self; 4] = [
        Self::ThirdAbove,
        Self::ThirdBelow,
        Self::SixthAbove,
        Self::SixthBelow,
    ];

    /// Scale steps from the melody.
    fn steps(self) -> i32 {
        match self {
            Self::ThirdAbove => 2,
            Self::ThirdBelow => -2,
            Self::SixthAbove => 5,
            Self::SixthBelow => -5,
        }
    }

    /// Where the harmony sits in the stereo field by default (above right, below left).
    pub fn pan(self) -> f32 {
        match self {
            Self::ThirdAbove | Self::SixthAbove => 0.35,
            Self::ThirdBelow | Self::SixthBelow => -0.35,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ThirdAbove => "third above",
            Self::ThirdBelow => "third below",
            Self::SixthAbove => "sixth above",
            Self::SixthBelow => "sixth below",
        }
    }
}

/// Default level of a harmony track against the lead.
pub const HARMONY_GAIN: f32 = 0.6;

/// `midi` moved `steps` degrees along `key`'s scale. A note outside the scale keeps its
/// distance above the degree below it.
pub fn diatonic(midi: u8, key: Key, steps: i32) -> u8 {
    let degrees = key.degrees();
    let relative = (i32::from(midi) - i32::from(key.tonic)).rem_euclid(12);
    let degree = degrees
        .iter()
        .rposition(|&d| i32::from(d) <= relative)
        .unwrap_or(0) as i32;
    let chromatic = relative - i32::from(degrees[degree as usize]);
    let target = degree + steps;
    let shifted = i32::from(midi) - relative
        + i32::from(degrees[target.rem_euclid(7) as usize])
        + 12 * target.div_euclid(7)
        + chromatic;
    shifted.clamp(0, 127) as u8
}

/// The harmony line of `notes`.
pub fn voice(notes: &[Note], key: Key, interval: Interval) -> Vec<Note> {
    notes
        .iter()
        .map(|n| Note {
            midi: diatonic(n.midi, key, interval.steps()),
            ..*n
        })
        .collect()
}

/// The melody of `source`: the vocal stem's reference notes, or a take's segmented notes.
pub fn source_notes(project: &Project, source: EditSource) -> Result<Vec<Note>, EditError> {
    let track = edit::source_track(project, source)?;
    Ok(match source {
        EditSource::VocalStem => project.analysis.reference_notes.clone(),
        EditSource::Take { .. } => melody::segment(track, edit::song_key(project)),
    })
}

/// Semitones to shift each frame of `track` by: the interval of the note it falls in (the
/// singer's own inflections ride along), 0 between notes and on unvoiced frames.
pub fn correction(track: &PitchTrack, notes: &[Note], key: Key, interval: Interval) -> Vec<f32> {
    let mut i = 0;
    (0..track.frames.len())
        .map(|f| {
            let t = track.time(f);
            while i < notes.len() && notes[i].end <= t {
                i += 1;
            }
            match (notes.get(i), track.frames[f]) {
                (Some(n), PitchFrame::Voiced { .. }) if n.start <= t => {
                    f32::from(diatonic(n.midi, key, interval.steps())) - f32::from(n.midi)
                }
                _ => 0.0,
            }
        })
        .collect()
}

/// Renders `source`'s harmony at `interval` from its audio (mono, song timeline, project rate).
pub fn render(
    project: &Project,
    source: EditSource,
    audio: &[f32],
    interval: Interval,
) -> Result<Vec<f32>, EditError> {
    let track = edit::source_track(project, source)?;
    let mut notes = source_notes(project, source)?;
    notes.sort_by(|a, b| a.start.total_cmp(&b.start));
    let curve = correction(track, &notes, edit::song_key(project), interval);
    Ok(shift::shift(
        audio,
        project.sample_rate,
        &curve,
        track.hop_seconds,
        0.0,
    ))
}

/// A Standard MIDI File of `source`'s melody, or of its harmony at `interval`.
pub fn melody_midi(
    project: &Project,
    source: EditSource,
    interval: Option<Interval>,
) -> Result<Vec<u8>, EditError> {
    let notes = source_notes(project, source)?;
    let notes = match interval {
        Some(i) => voice(&notes, edit::song_key(project), i),
        None => notes,
    };
    let notes: Vec<MidiNote> = notes
        .iter()
        .map(|n| MidiNote {
            start: n.start,
            end: n.end,
            pitch: n.midi,
            velocity: 100,
        })
        .collect();
    Ok(midi::smf_notes(&notes, 0, project.analysis.tempo.bpm))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::Mode;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    const C_MAJOR: Key = Key {
        tonic: 0,
        mode: Mode::Major,
    };
    const A_MINOR: Key = Key {
        tonic: 9,
        mode: Mode::Minor,
    };
    const E4: u8 = 64;

    #[test]
    fn thirds_and_sixths_follow_the_scale() {
        let up = |m, i: Interval| diatonic(m, C_MAJOR, i.steps());
        assert_eq!(up(E4, Interval::ThirdAbove), 67, "E → G");
        assert_eq!(up(E4, Interval::ThirdBelow), 60, "E → C");
        assert_eq!(up(E4, Interval::SixthAbove), 72, "E → C above");
        assert_eq!(up(E4, Interval::SixthBelow), 55, "E → G below");
        // Diatonic, so the size of the third depends on the degree: D → F is minor, C → E major.
        assert_eq!(up(62, Interval::ThirdAbove), 65);
        assert_eq!(up(60, Interval::ThirdAbove), 64);
        // B → D crosses the octave.
        assert_eq!(up(71, Interval::ThirdAbove), 74);
        // A minor: A → C, E → G, G → B.
        assert_eq!(diatonic(69, A_MINOR, 2), 72);
        assert_eq!(diatonic(76, A_MINOR, 2), 79);
        assert_eq!(diatonic(67, A_MINOR, 2), 71);
        // Out of scale: F♯ keeps its half step over F, F → A, so F♯ → A♯.
        assert_eq!(up(66, Interval::ThirdAbove), 70);
    }

    #[test]
    fn correction_shifts_voiced_frames_inside_notes() {
        let hop = 0.01;
        let track = PitchTrack {
            hop_seconds: hop,
            frames: (0..100)
                .map(|i| {
                    if (20..80).contains(&i) && i != 50 {
                        PitchFrame::Voiced { midi: 64.2 }
                    } else {
                        PitchFrame::Unvoiced
                    }
                })
                .collect(),
        };
        let notes = [Note {
            start: 0.2,
            end: 0.8,
            midi: E4,
        }];
        let c = correction(&track, &notes, C_MAJOR, Interval::ThirdAbove);
        assert_eq!(c[30], 3.0, "E → G is a minor third");
        assert_eq!(c[50], 0.0, "unvoiced");
        assert_eq!(c[10], 0.0);
        assert_eq!(c[90], 0.0);
        let below = correction(&track, &notes, C_MAJOR, Interval::SixthBelow);
        assert_eq!(below[30], -9.0);
    }

    #[test]
    fn harmony_notes_export_as_midi() {
        let notes = [Note {
            start: 0.0,
            end: 0.5,
            midi: E4,
        }];
        let v = voice(&notes, C_MAJOR, Interval::ThirdAbove);
        assert_eq!(v[0].midi, 67);
        let midi = midi::smf_notes(
            &[MidiNote {
                start: 0.0,
                end: 0.5,
                pitch: v[0].midi,
                velocity: 100,
            }],
            0,
            120.0,
        );
        assert_eq!(&midi[..4], b"MThd");
        assert!(midi.windows(3).any(|w| w == [0x90, 67, 100]));
    }
}
