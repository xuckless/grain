//! Project model shared by every host, plus the song analysis run on import (AGENTS.md §4.4–4.5).

use crate::audio::StereoAudio;
use crate::key::{self, KeyCandidate};
use crate::melody::{self, Note};
use crate::onset;
use crate::peaks::peaks;
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Analysis {
    pub tempo: Tempo,
    /// Best three keys, best first.
    pub keys: Vec<KeyCandidate>,
    pub reference_source: ReferenceSource,
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
    pub tracks: Vec<Track>,
    pub analysis: Analysis,
}

/// Song audio as the host hands it to [`build`].
pub enum SongAudio {
    Unseparated { mix: StereoAudio },
    Separated { stems: Vec<(Stem, StereoAudio)> },
}

pub fn build(song: SongSource, audio: &SongAudio) -> Project {
    let tracks: Vec<(TrackSource, &StereoAudio)> = match audio {
        SongAudio::Unseparated { mix } => vec![(TrackSource::Mix, mix)],
        SongAudio::Separated { stems } => stems
            .iter()
            .map(|(stem, a)| (TrackSource::Stem { stem: *stem }, a))
            .collect(),
    };
    let first = tracks[0].1;
    Project {
        version: PROJECT_VERSION,
        song,
        sample_rate: first.sample_rate,
        frames: first.len() as u32,
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
        analysis: analyze(audio),
    }
}

fn analyze(audio: &SongAudio) -> Analysis {
    let sr = sample_rate(audio);
    let mix = mono_sum(audio, |_| true);
    let tempo = tempo::estimate(&onset::envelope(&mix, sr));
    let keys = key::estimate(&mono_sum(audio, |s| s != Stem::Vocals), sr);
    let reference_pitch = pitch::track(&mono_sum(audio, |s| s == Stem::Vocals), sr);
    let reference_notes = melody::segment(&reference_pitch, keys[0].key);
    Analysis {
        tempo,
        keys,
        reference_source: match audio {
            SongAudio::Unseparated { .. } => ReferenceSource::FullMix,
            SongAudio::Separated { .. } => ReferenceSource::VocalStem,
        },
        reference_pitch,
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

/// Scores a sung take (same sample rate as the song) against the reference vocal (AGENTS.md §5).
pub fn score_take(
    project: &Project,
    audio: &SongAudio,
    take: &StereoAudio,
    settings: ScoreSettings,
) -> ScoreReport {
    let sr = sample_rate(audio);
    assert_eq!(
        take.sample_rate, sr,
        "resample the take to the song rate first"
    );
    let take_mono = take.to_mono();
    let vocals = mono_sum(audio, |s| s == Stem::Vocals);
    let offset = score::estimate_offset(
        &onset::envelope(&vocals, sr),
        &onset::envelope(&take_mono, sr),
    );
    score::score(
        &project.analysis.reference_pitch,
        &project.analysis.reference_notes,
        &pitch::track(&take_mono, sr),
        offset,
        settings,
    )
}

/// Player tracks for a project's audio, in project track order.
pub fn player_tracks(project: &Project, audio: &SongAudio) -> Vec<PlayerTrack> {
    let sources: Vec<&StereoAudio> = match audio {
        SongAudio::Unseparated { mix } => vec![mix],
        SongAudio::Separated { stems } => stems.iter().map(|(_, a)| a).collect(),
    };
    project
        .tracks
        .iter()
        .zip(sources)
        .map(|(t, a)| PlayerTrack {
            left: a.left.clone().into(),
            right: a.right.clone().into(),
            mix: t.mix,
        })
        .collect()
}
