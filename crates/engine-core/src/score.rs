//! Offline performance scoring (AGENTS.md §5).

use crate::melody::Note;
use crate::note::fold_octave;
use crate::pitch::{PitchFrame, PitchTrack};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Search range for the take's offset against the reference.
const MAX_OFFSET_SECONDS: f64 = 10.0;
/// A note's onset is searched from this long before its start.
const ONSET_LEAD_SECONDS: f64 = 0.25;
/// Onset error at which the timing score of a note reaches zero.
const TIMING_ZERO_MS: f32 = 300.0;
/// Pitch spread at which the stability score of a note reaches zero.
const STABILITY_ZERO_CENTS: f32 = 50.0;
/// Notes shorter than this are not analyzed for vibrato.
const VIBRATO_MIN_SECONDS: f64 = 0.3;
/// A 4–8 Hz component at least this large (cents) counts as vibrato.
const VIBRATO_MIN_EXTENT: f32 = 15.0;

/// Weights of the overall score: pitch, timing, stability, coverage.
pub const SCORE_WEIGHTS: [f32; 4] = [0.4, 0.2, 0.2, 0.2];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Difficulty {
    Easy,
    Medium,
    Hard,
}

impl Difficulty {
    pub fn tolerance_cents(self) -> f32 {
        match self {
            Difficulty::Easy => 50.0,
            Difficulty::Medium => 35.0,
            Difficulty::Hard => 25.0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum OctaveMode {
    /// Octave errors count as unison (e.g. a man singing a female part).
    Fold,
    Strict,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ScoreSettings {
    pub difficulty: Difficulty,
    pub octave: OctaveMode,
}

impl Default for ScoreSettings {
    fn default() -> Self {
        Self {
            difficulty: Difficulty::Medium,
            octave: OctaveMode::Fold,
        }
    }
}

/// Per reference frame comparison.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum FrameComparison {
    Hit {
        cents: f32,
    },
    Miss {
        cents: f32,
    },
    /// The reference sings, the user does not.
    ReferenceOnly,
    /// The user sings where the reference is silent.
    UserOnly,
    Silent,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum Vibrato {
    Absent,
    Present { rate_hz: f32, extent_cents: f32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum NoteResult {
    Sung {
        mean_cents: f32,
        /// Fraction of the note's duration sung within tolerance.
        hit_ratio: f32,
        onset_ms: f32,
        stability_cents: f32,
        vibrato: Vibrato,
    },
    Missed,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct NoteReport {
    pub note: Note,
    pub result: NoteResult,
}

/// Scores in 0–100.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Scores {
    pub overall: f32,
    pub pitch: f32,
    pub timing: f32,
    pub stability: f32,
    pub coverage: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ScoreReport {
    pub settings: ScoreSettings,
    /// Time of the take minus time of the reference for the same musical moment.
    pub offset_seconds: f64,
    pub hop_seconds: f64,
    /// One entry per reference pitch frame.
    pub frames: Vec<FrameComparison>,
    /// The user's pitch track shifted onto the reference timeline.
    pub user_pitch: PitchTrack,
    pub notes: Vec<NoteReport>,
    pub scores: Scores,
}

/// Offset (seconds) at which the take's pitch contour best agrees with the reference: each
/// frame where both are voiced scores 1 at unison (octave-folded), falling to 0 at 100 cents.
pub fn estimate_offset(reference: &PitchTrack, take: &PitchTrack) -> f64 {
    let hop = reference.hop_seconds;
    let max_lag = (MAX_OFFSET_SECONDS / hop) as isize;
    let midi = |f: &PitchFrame| match f {
        PitchFrame::Voiced { midi } => Some(*midi),
        PitchFrame::Unvoiced => None,
    };
    let r: Vec<(usize, f32)> = reference
        .frames
        .iter()
        .enumerate()
        .filter_map(|(i, f)| Some((i, midi(f)?)))
        .collect();
    let u: Vec<Option<f32>> = take.frames.iter().map(midi).collect();
    let agreement = |lag: isize| -> f32 {
        r.iter()
            .filter_map(|(i, rm)| {
                let j = *i as isize + lag;
                let um = (*u.get(usize::try_from(j).ok()?)?)?;
                Some((1.0 - fold_octave(100.0 * (um - rm)).abs() / 100.0).max(0.0))
            })
            .sum()
    };
    let best = (-max_lag..=max_lag)
        .map(|lag| (lag, agreement(lag)))
        .max_by(|a, b| a.1.total_cmp(&b.1).then(b.0.abs().cmp(&a.0.abs())))
        .map(|(lag, _)| lag)
        .unwrap_or(0);
    best as f64 * hop
}

pub fn score(
    reference: &PitchTrack,
    notes: &[Note],
    take: &PitchTrack,
    offset_seconds: f64,
    settings: ScoreSettings,
) -> ScoreReport {
    assert!(
        (reference.hop_seconds - take.hop_seconds).abs() < 1e-9,
        "reference and take must come from the same pitch tracker"
    );
    let hop = reference.hop_seconds;
    let shift = (offset_seconds / hop).round() as isize;
    let user_pitch = PitchTrack {
        hop_seconds: hop,
        frames: (0..reference.frames.len() as isize)
            .map(|i| {
                let j = i + shift;
                if j >= 0 && (j as usize) < take.frames.len() {
                    take.frames[j as usize]
                } else {
                    PitchFrame::Unvoiced
                }
            })
            .collect(),
    };
    let tol = settings.difficulty.tolerance_cents();
    let error = |midi: f32, reference_midi: f32| {
        let c = 100.0 * (midi - reference_midi);
        match settings.octave {
            OctaveMode::Fold => fold_octave(c),
            OctaveMode::Strict => c,
        }
    };

    let frames: Vec<FrameComparison> = reference
        .frames
        .iter()
        .zip(&user_pitch.frames)
        .map(|(r, u)| match (r, u) {
            (PitchFrame::Voiced { midi: rm }, PitchFrame::Voiced { midi: um }) => {
                let c = error(*um, *rm);
                if c.abs() <= tol {
                    FrameComparison::Hit { cents: c }
                } else {
                    FrameComparison::Miss { cents: c }
                }
            }
            (PitchFrame::Voiced { .. }, PitchFrame::Unvoiced) => FrameComparison::ReferenceOnly,
            (PitchFrame::Unvoiced, PitchFrame::Voiced { .. }) => FrameComparison::UserOnly,
            (PitchFrame::Unvoiced, PitchFrame::Unvoiced) => FrameComparison::Silent,
        })
        .collect();

    let notes: Vec<NoteReport> = notes
        .iter()
        .map(|note| NoteReport {
            note: *note,
            result: score_note(note, &user_pitch, tol, &error),
        })
        .collect();

    let count = |f: fn(&FrameComparison) -> bool| frames.iter().filter(|c| f(c)).count() as f32;
    let hits = count(|c| matches!(c, FrameComparison::Hit { .. }));
    let compared = hits + count(|c| matches!(c, FrameComparison::Miss { .. }));
    let reference_voiced = compared + count(|c| matches!(c, FrameComparison::ReferenceOnly));
    let sung: Vec<(f32, f32)> = notes
        .iter()
        .filter_map(|n| match n.result {
            NoteResult::Sung {
                onset_ms,
                stability_cents,
                ..
            } => Some((onset_ms, stability_cents)),
            NoteResult::Missed => None,
        })
        .collect();
    let mean = |values: Vec<f32>| {
        if values.is_empty() {
            0.0
        } else {
            values.iter().sum::<f32>() / values.len() as f32
        }
    };
    let ratio = |a: f32, b: f32| if b > 0.0 { 100.0 * a / b } else { 0.0 };
    let pitch = ratio(hits, compared);
    let coverage = ratio(compared, reference_voiced);
    let timing = mean(
        sung.iter()
            .map(|(o, _)| 100.0 * (1.0 - o.abs() / TIMING_ZERO_MS).max(0.0))
            .collect(),
    );
    let stability = mean(
        sung.iter()
            .map(|(_, s)| 100.0 * (1.0 - s / STABILITY_ZERO_CENTS).max(0.0))
            .collect(),
    );
    let [wp, wt, ws, wc] = SCORE_WEIGHTS;
    ScoreReport {
        settings,
        offset_seconds,
        hop_seconds: hop,
        frames,
        user_pitch,
        notes,
        scores: Scores {
            overall: wp * pitch + wt * timing + ws * stability + wc * coverage,
            pitch,
            timing,
            stability,
            coverage,
        },
    }
}

fn score_note(
    note: &Note,
    user: &PitchTrack,
    tol: f32,
    error: &dyn Fn(f32, f32) -> f32,
) -> NoteResult {
    let hop = user.hop_seconds;
    let target = note.midi as f32;
    let frame_of = |t: f64| (t / hop).round().max(0.0) as usize;
    let (first, last) = (
        frame_of(note.start),
        frame_of(note.end).min(user.frames.len()),
    );
    let voiced_error = |i: usize| match user.frames.get(i) {
        Some(PitchFrame::Voiced { midi }) => Some(error(*midi, target)),
        _ => None,
    };

    let onset = (frame_of(note.start - ONSET_LEAD_SECONDS)..last)
        .find(|i| voiced_error(*i).is_some_and(|e| e.abs() <= 2.0 * tol));
    let Some(onset) = onset else {
        return NoteResult::Missed;
    };
    let errors: Vec<f32> = (first..last).filter_map(voiced_error).collect();
    if errors.is_empty() {
        return NoteResult::Missed;
    }
    let total = (last - first).max(1) as f32;
    let hit_ratio = errors.iter().filter(|e| e.abs() <= tol).count() as f32 / total;
    let mean_cents = errors.iter().sum::<f32>() / errors.len() as f32;
    let centered: Vec<f32> = errors.iter().map(|e| e - mean_cents).collect();
    let (vibrato, residual) = fit_vibrato(&centered, hop, note.end - note.start);
    let stability_cents =
        (residual.iter().map(|v| v * v).sum::<f32>() / residual.len() as f32).sqrt();
    NoteResult::Sung {
        mean_cents,
        hit_ratio,
        onset_ms: ((onset as f64 * hop - note.start) * 1000.0) as f32,
        stability_cents,
        vibrato,
    }
}

/// Finds the strongest 4–8 Hz sinusoid in a mean-removed cents contour and returns it with
/// the residual (contour minus that sinusoid, when vibrato is present).
fn fit_vibrato(contour: &[f32], hop: f64, seconds: f64) -> (Vibrato, Vec<f32>) {
    if seconds < VIBRATO_MIN_SECONDS {
        return (Vibrato::Absent, contour.to_vec());
    }
    let n = contour.len() as f32;
    let tau = 2.0 * std::f32::consts::PI;
    let best = (0..=16)
        .map(|k| 4.0 + 0.25 * k as f32)
        .map(|rate| {
            let (re, im) = contour
                .iter()
                .enumerate()
                .fold((0.0, 0.0), |(re, im), (i, v)| {
                    let phase = tau * rate * i as f32 * hop as f32;
                    (re + v * phase.cos(), im + v * phase.sin())
                });
            (rate, 2.0 * re / n, 2.0 * im / n)
        })
        .max_by(|a, b| (a.1.hypot(a.2)).total_cmp(&b.1.hypot(b.2)));
    match best {
        Some((rate, a, b)) if a.hypot(b) >= VIBRATO_MIN_EXTENT => {
            let residual = contour
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let phase = tau * rate * i as f32 * hop as f32;
                    v - (a * phase.cos() + b * phase.sin())
                })
                .collect();
            (
                Vibrato::Present {
                    rate_hz: rate,
                    extent_cents: a.hypot(b),
                },
                residual,
            )
        }
        _ => (Vibrato::Absent, contour.to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::melody::segment;
    use crate::melody::tests::{synthetic_track, C_MAJOR};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn reference() -> (PitchTrack, Vec<Note>) {
        let track = synthetic_track(
            &[
                (Some(60.0), 0.6),
                (None, 0.2),
                (Some(64.0), 0.6),
                (None, 0.2),
                (Some(67.0), 0.6),
            ],
            0.0,
        );
        let notes = segment(&track, C_MAJOR);
        assert_eq!(notes.len(), 3);
        (track, notes)
    }

    #[test]
    fn perfect_take_scores_full_marks() {
        let (r, notes) = reference();
        let report = score(&r, &notes, &r, 0.0, ScoreSettings::default());
        assert!(report.scores.pitch > 99.0 && report.scores.coverage > 99.0);
        assert!(report.scores.timing > 95.0 && report.scores.stability > 95.0);
        assert!(report.scores.overall > 97.0);
    }

    #[test]
    fn octave_down_take_is_folded_or_penalized() {
        let (r, notes) = reference();
        let low = synthetic_track(
            &[
                (Some(48.0), 0.6),
                (None, 0.2),
                (Some(52.0), 0.6),
                (None, 0.2),
                (Some(55.0), 0.6),
            ],
            0.0,
        );
        let folded = score(&r, &notes, &low, 0.0, ScoreSettings::default());
        assert!(folded.scores.pitch > 99.0);
        let strict = score(
            &r,
            &notes,
            &low,
            0.0,
            ScoreSettings {
                octave: OctaveMode::Strict,
                ..ScoreSettings::default()
            },
        );
        assert!(strict.scores.pitch < 1.0);
        assert!(strict.notes.iter().all(|n| n.result == NoteResult::Missed));
    }

    #[test]
    fn flat_take_misses_hard_tolerance_but_hits_easy() {
        let (r, notes) = reference();
        let flat = synthetic_track(
            &[
                (Some(59.6), 0.6),
                (None, 0.2),
                (Some(63.6), 0.6),
                (None, 0.2),
                (Some(66.6), 0.6),
            ],
            0.0,
        );
        let easy = score(
            &r,
            &notes,
            &flat,
            0.0,
            ScoreSettings {
                difficulty: Difficulty::Easy,
                ..Default::default()
            },
        );
        let hard = score(
            &r,
            &notes,
            &flat,
            0.0,
            ScoreSettings {
                difficulty: Difficulty::Hard,
                ..Default::default()
            },
        );
        assert!(easy.scores.pitch > 99.0);
        assert!(hard.scores.pitch < 1.0);
        match easy.notes[0].result {
            NoteResult::Sung { mean_cents, .. } => assert!((mean_cents + 40.0).abs() < 1.0),
            NoteResult::Missed => panic!("note should be sung"),
        }
    }

    #[test]
    fn detects_vibrato_and_excludes_it_from_stability() {
        let (r, notes) = reference();
        let wobbly = synthetic_track(
            &[
                (Some(60.0), 0.6),
                (None, 0.2),
                (Some(64.0), 0.6),
                (None, 0.2),
                (Some(67.0), 0.6),
            ],
            40.0,
        );
        let report = score(&r, &notes, &wobbly, 0.0, ScoreSettings::default());
        match report.notes[0].result {
            NoteResult::Sung {
                vibrato:
                    Vibrato::Present {
                        rate_hz,
                        extent_cents,
                    },
                stability_cents,
                ..
            } => {
                assert!((rate_hz - 5.5).abs() < 0.3, "rate {rate_hz}");
                assert!((extent_cents - 40.0).abs() < 8.0, "extent {extent_cents}");
                assert!(stability_cents < 12.0, "stability {stability_cents}");
            }
            other => panic!("expected vibrato, got {other:?}"),
        }
    }

    #[test]
    fn late_take_is_realigned_by_offset() {
        let (r, notes) = reference();
        let mut late = synthetic_track(&[(None, 1.5)], 0.0);
        late.frames.extend(r.frames.iter().copied());
        let report = score(&r, &notes, &late, 1.5, ScoreSettings::default());
        assert!(report.scores.overall > 97.0);
        assert!(
            score(&r, &notes, &late, 0.0, ScoreSettings::default())
                .scores
                .overall
                < 50.0
        );
    }

    #[test]
    fn offset_estimation_recovers_shift() {
        let (r, _) = reference();
        let mut late = synthetic_track(&[(None, 1.23)], 0.0);
        late.frames.extend(r.frames.iter().copied());
        assert!((estimate_offset(&r, &late) - 1.23).abs() < 1e-6);
        let early = PitchTrack {
            hop_seconds: 0.01,
            frames: r.frames[40..].to_vec(),
        };
        assert!((estimate_offset(&r, &early) + 0.40).abs() < 1e-6);
    }
}
