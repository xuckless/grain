//! Project model shared by every host, plus the song analysis run on import (AGENTS.md §4.4–4.5).

use crate::audio::StereoAudio;
use crate::key::{self, KeyCandidate};
use crate::lyrics::ProjectLyrics;
use crate::melody::{self, Note};
use crate::onset;
use crate::peaks::{peaks, SAMPLES_PER_PEAK};
use crate::pitch::{self, PitchTrack};
use crate::player::{MixParams, PlayerTrack};
use crate::take::Take;
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

/// Below this level (dB relative to the mix) the non-vocal stems count as silent: the song is
/// a cappella, so an instrumental backing would leave a singer with nothing to hear.
pub const ACCOMPANIMENT_ABSENT_DB: f32 = -30.0;

/// How much the song carries besides the vocal, measured once from the stems at import (or at
/// load for projects saved before it existed). It decides the default recording backing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum Accompaniment {
    /// Saved before this was measured; the host measures it when it loads the stems.
    #[default]
    Unmeasured,
    /// No separation: the vocal can't be taken out of the mix.
    NoStems,
    /// The non-vocal stems carry real energy, `relative_db` below the mix.
    Present { relative_db: f32 },
    /// The non-vocal stems are near-silent (more than 30 dB below the mix).
    Absent { relative_db: f32 },
}

/// Measures [`Accompaniment`] from a song's stems.
pub fn measure_accompaniment(audio: &SongAudio) -> Accompaniment {
    let SongAudio::Separated { .. } = audio else {
        return Accompaniment::NoStems;
    };
    let db = |x: &[f32]| {
        let energy: f64 = x.iter().map(|v| f64::from(v * v)).sum();
        (10.0 * (energy / x.len().max(1) as f64 + 1e-12).log10()) as f32
    };
    let relative_db = db(&mono_sum(audio, |s| s != Stem::Vocals)) - db(&mono_sum(audio, |_| true));
    if relative_db < ACCOMPANIMENT_ABSENT_DB {
        Accompaniment::Absent { relative_db }
    } else {
        Accompaniment::Present { relative_db }
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
    #[serde(default)]
    pub accompaniment: Accompaniment,
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
    /// Sung takes, newest last (see [`crate::take`]).
    #[serde(default)]
    pub takes: Vec<Take>,
    /// Karaoke lyrics, fetched on demand (see [`crate::lyrics`]).
    #[serde(default)]
    pub lyrics: ProjectLyrics,
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
        takes: Vec::new(),
        lyrics: ProjectLyrics::NotFetched,
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
        accompaniment: measure_accompaniment(audio),
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

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum RestoreError {
    #[error("the project has {expected} tracks but {found} were stored")]
    TrackCount { expected: usize, found: usize },
    #[error("a mix track inside a separated project")]
    MixInSeparated,
}

/// Rebuilds a saved project's song audio from its stored tracks (in project track order), and
/// measures the accompaniment of projects saved before it existed.
pub fn restore_audio(
    project: &mut Project,
    tracks: Vec<StereoAudio>,
) -> Result<SongAudio, RestoreError> {
    if tracks.len() != project.tracks.len() {
        return Err(RestoreError::TrackCount {
            expected: project.tracks.len(),
            found: tracks.len(),
        });
    }
    let audio = match project.tracks.as_slice() {
        [only] if only.source == TrackSource::Mix => SongAudio::Unseparated {
            mix: tracks.into_iter().next().expect("one track"),
        },
        _ => SongAudio::Separated {
            stems: project
                .tracks
                .iter()
                .zip(tracks)
                .map(|(t, a)| match t.source {
                    TrackSource::Stem { stem } => Ok((stem, a)),
                    TrackSource::Mix => Err(RestoreError::MixInSeparated),
                })
                .collect::<Result<_, _>>()?,
        },
    };
    if project.analysis.accompaniment == Accompaniment::Unmeasured {
        project.analysis.accompaniment = measure_accompaniment(&audio);
    }
    Ok(audio)
}

/// Stores a player track's mix in the project. Player tracks are the song's tracks, then its
/// takes (see [`crate::take::player_index`]); unknown indices are ignored.
pub fn set_mix(project: &mut Project, player_track: u32, params: MixParams) {
    let i = player_track as usize;
    let songs = project.tracks.len();
    if let Some(t) = project.tracks.get_mut(i) {
        t.mix = params;
    } else if let Some(t) = i.checked_sub(songs).and_then(|j| project.takes.get_mut(j)) {
        t.mix = params;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn stereo(level: f32, frames: usize) -> StereoAudio {
        let signal: Vec<f32> = (0..frames)
            .map(|i| level * (i as f32 * 0.05).sin())
            .collect();
        StereoAudio {
            sample_rate: 8000,
            left: signal.clone(),
            right: signal,
        }
    }

    fn separated(other: f32) -> SongAudio {
        SongAudio::Separated {
            stems: vec![
                (Stem::Vocals, stereo(0.5, 8000)),
                (Stem::Other, stereo(other, 8000)),
            ],
        }
    }

    #[test]
    fn accompaniment_is_absent_below_minus_30_db() {
        assert!(matches!(
            measure_accompaniment(&separated(0.3)),
            Accompaniment::Present { .. }
        ));
        assert!(
            matches!(measure_accompaniment(&separated(0.005)), Accompaniment::Absent { relative_db } if relative_db < -30.0)
        );
        let mix = SongAudio::Unseparated {
            mix: stereo(0.5, 8000),
        };
        assert_eq!(measure_accompaniment(&mix), Accompaniment::NoStems);
    }

    #[test]
    fn a_saved_project_restores_its_audio_and_mixes() {
        let audio = separated(0.005);
        let mut project = build(
            SongSource::File { name: "s".into() },
            &audio,
            TrackedPitch::yin(&reference_signal(&audio), 8000),
        );
        project.analysis.accompaniment = Accompaniment::Unmeasured;
        let tracks = track_audio(&audio).into_iter().cloned().collect();
        let restored = restore_audio(&mut project, tracks).unwrap();
        assert_eq!(track_audio(&restored).len(), 2);
        assert!(matches!(
            project.analysis.accompaniment,
            Accompaniment::Absent { .. }
        ));
        assert!(matches!(
            restore_audio(&mut project, vec![]),
            Err(RestoreError::TrackCount {
                expected: 2,
                found: 0
            })
        ));

        let muted = MixParams {
            mode: crate::player::TrackMode::Muted,
            ..MixParams::default()
        };
        set_mix(&mut project, 1, muted);
        assert_eq!(project.tracks[1].mix, muted);
        set_mix(&mut project, 9, MixParams::default()); // no such track: ignored
    }
}
