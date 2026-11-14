//! Project model shared by every host, plus the song analysis run on import (AGENTS.md §4.4–4.5).

use crate::audio::StereoAudio;
use crate::key::{self, KeyCandidate};
use crate::melody::{self, Note};
use crate::onset;
use crate::peaks::{peaks, SAMPLES_PER_PEAK};
use crate::pitch::{self, PitchTrack};
use crate::player::{MixParams, PlayerTrack};
use crate::score::{self, ScoreReport, ScoreSettings};
use crate::tempo::{self, Tempo};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

pub const PROJECT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Stem {
    Drums,
    Bass,
    Other,
    Vocals,
    Guitar,
    Piano,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum TrackSource {
    /// The original mix (used when no separation model is installed).
    Mix,
    Stem {
        stem: Stem,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum SongSource {
    File { name: String },
    YouTube { url: String, title: String },
}

/// Which signal the reference vocal pitch was tracked on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ReferenceSource {
    VocalStem,
    /// No separation available: tracked on the full mix, so expect errors.
    FullMix,
}

/// The pitch tracker behind a project's reference melody. Takes are always tracked with the
/// same one, so reference and take are comparable (AGENTS.md §5). `Yin`: built in, used where the
/// RMVPE model is unavailable (web, or no weights) and by projects saved before RMVPE.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum PitchTracker {
    #[default]
    Yin,
    Rmvpe,
}

/// A pitch contour and the tracker that produced it.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackedPitch {
    pub tracker: PitchTracker,
    pub track: PitchTrack,
}

impl TrackedPitch {
    /// The built-in tracker ([`pitch::track`]).
    pub fn yin(signal: &[f32], sample_rate: u32) -> Self {
        Self {
            tracker: PitchTracker::Yin,
            track: pitch::track(signal, sample_rate),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Analysis {
    pub tempo: Tempo,
    /// Best three keys, best first.
    pub keys: Vec<KeyCandidate>,
    pub reference_source: ReferenceSource,
    #[serde(default)]
    pub pitch_tracker: PitchTracker,
    pub reference_pitch: PitchTrack,
    pub reference_notes: Vec<Note>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Track {
    pub name: String,
    pub source: TrackSource,
    pub mix: MixParams,
    /// Waveform overview of the mono sum (see `peaks::peaks`).
    pub peaks: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Project {
    pub version: u32,
    pub song: SongSource,
    pub sample_rate: u32,
    pub frames: u32,
    /// Samples summarized by each `[min, max]` pair in `Track::peaks`.
    pub samples_per_peak: u32,
    pub tracks: Vec<Track>,
    pub analysis: Analysis,
}

/// Which separation an import runs (AGENTS.md §4.2). `Fast`: HTDemucs alone. `BestVocals`: the
/// RoFormer vocal cascade: an opt-in quality option for songs with real accompaniment, several
/// times slower, with a cleaner vocal stem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum SeparationQuality {
    Fast,
    BestVocals,
}

/// Progress of one import, reported as it advances.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum ImportStage {
    Decoding,
    Separating { segment: u32, of: u32 },
    Analyzing,
    Caching,
}

/// A saved project as listed on the Home screen. `id` is the host's handle for opening it
/// (the stem-cache key on desktop).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProjectSummary {
    pub id: String,
    pub song: SongSource,
    pub duration_seconds: f64,
    pub bpm: f64,
    /// Best key's display name, if a key was found.
    pub key: Option<String>,
    pub tracks: u32,
    /// Last saved, milliseconds since the Unix epoch.
    pub saved_ms: f64,
}

impl ProjectSummary {
    pub fn new(id: String, project: &Project, saved_ms: f64) -> Self {
        Self {
            id,
            song: project.song.clone(),
            duration_seconds: f64::from(project.frames) / f64::from(project.sample_rate),
            bpm: project.analysis.tempo.bpm,
            key: project.analysis.keys.first().map(|k| k.label.clone()),
            tracks: project.tracks.len() as u32,
            saved_ms,
        }
    }
}

/// Song audio as the host hands it to [`build`].
pub enum SongAudio {
    Unseparated { mix: StereoAudio },
    Separated { stems: Vec<(Stem, StereoAudio)> },
}

/// Mono signal the reference melody is tracked on: the vocal stem, or the mix when unseparated.
pub fn reference_signal(audio: &SongAudio) -> Vec<f32> {
    mono_sum(audio, |s| s == Stem::Vocals)
}

/// Builds a song's project. `reference` is [`reference_signal`] tracked by the host's tracker.
pub fn build(song: SongSource, audio: &SongAudio, reference: TrackedPitch) -> Project {
    let sources: Vec<TrackSource> = match audio {
        SongAudio::Unseparated { .. } => vec![TrackSource::Mix],
        SongAudio::Separated { stems } => stems
            .iter()
            .map(|(stem, _)| TrackSource::Stem { stem: *stem })
            .collect(),
    };
    let tracks: Vec<(TrackSource, &StereoAudio)> =
        sources.into_iter().zip(track_audio(audio)).collect();
    let first = tracks[0].1;
    Project {
        version: PROJECT_VERSION,
        song,
        sample_rate: first.sample_rate,
        frames: first.len() as u32,
        samples_per_peak: SAMPLES_PER_PEAK as u32,
        tracks: tracks
            .iter()
            .map(|(source, a)| Track {
                name: match source {
                    TrackSource::Mix => "Mix".to_string(),
                    TrackSource::Stem { stem } => format!("{stem:?}"),
                },
                source: *source,
                mix: MixParams::default(),
                peaks: peaks(&a.to_mono()),
            })
            .collect(),
        analysis: analyze(audio, reference),
    }
}

fn analyze(audio: &SongAudio, reference: TrackedPitch) -> Analysis {
    let sr = sample_rate(audio);
    let mix = mono_sum(audio, |_| true);
    let tempo = tempo::estimate(&onset::envelope(&mix, sr));
    let keys = key::estimate(&mono_sum(audio, |s| s != Stem::Vocals), sr);
    let reference_notes = melody::segment(&reference.track, keys[0].key);
    Analysis {
        tempo,
        keys,
        reference_source: match audio {
            SongAudio::Unseparated { .. } => ReferenceSource::FullMix,
            SongAudio::Separated { .. } => ReferenceSource::VocalStem,
        },
        pitch_tracker: reference.tracker,
        reference_pitch: reference.track,
        reference_notes,
    }
}

fn sample_rate(audio: &SongAudio) -> u32 {
    match audio {
        SongAudio::Unseparated { mix } => mix.sample_rate,
        SongAudio::Separated { stems } => stems[0].1.sample_rate,
    }
}

/// Mono sum of the stems selected by `include`; an unseparated song always yields its mix.
fn mono_sum(audio: &SongAudio, include: impl Fn(Stem) -> bool) -> Vec<f32> {
    match audio {
        SongAudio::Unseparated { mix } => mix.to_mono(),
        SongAudio::Separated { stems } => {
            let mut sum = vec![0.0f32; stems[0].1.len()];
            for (_, a) in stems.iter().filter(|(s, _)| include(*s)) {
                for (acc, (l, r)) in sum.iter_mut().zip(a.left.iter().zip(&a.right)) {
                    *acc += 0.5 * (l + r);
                }
            }
            sum
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ScoreError {
    #[error("the take was tracked with {take:?} but the song's reference with {reference:?}")]
    TrackerMismatch {
        reference: PitchTracker,
        take: PitchTracker,
    },
}

/// Scores a sung take against the reference vocal (AGENTS.md §5). The host tracks the take with
/// the project's [`Analysis::pitch_tracker`].
pub fn score_take(
    project: &Project,
    take: &TrackedPitch,
    settings: ScoreSettings,
) -> Result<ScoreReport, ScoreError> {
    let reference_tracker = project.analysis.pitch_tracker;
    if take.tracker != reference_tracker {
        return Err(ScoreError::TrackerMismatch {
            reference: reference_tracker,
            take: take.tracker,
        });
    }
    let reference = &project.analysis.reference_pitch;
    let offset = score::estimate_offset(reference, &take.track);
    Ok(score::score(
        reference,
        &project.analysis.reference_notes,
        &take.track,
        offset,
        settings,
    ))
}

/// Audio of each project track, in project track order.
pub fn track_audio(audio: &SongAudio) -> Vec<&StereoAudio> {
    match audio {
        SongAudio::Unseparated { mix } => vec![mix],
        SongAudio::Separated { stems } => stems.iter().map(|(_, a)| a).collect(),
    }
}

/// Player tracks for a project's audio, in project track order.
pub fn player_tracks(project: &Project, audio: &SongAudio) -> Vec<PlayerTrack> {
    project
        .tracks
        .iter()
        .zip(track_audio(audio))
        .map(|(t, a)| PlayerTrack {
            left: a.left.clone().into(),
            right: a.right.clone().into(),
            mix: t.mix,
        })
        .collect()
}
