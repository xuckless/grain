//! Produce (AGENTS.md §8): what both hosts share around the vocal chain, denoising and
//! generation. Every result becomes a new take (non-destructive) that keeps how it was made.

use crate::chain::{self, ChainPreset, ChainReport, ChainSettings, Measures};
use crate::generate::{GenerationRequest, GenerationTask, GENERATION_MODEL};
use crate::project::Project;
use crate::take::TakeAdded;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A preset's settings for a source, with the measures they came from.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChainPick {
    pub settings: ChainSettings,
    pub measures: Measures,
}

/// A vocal chain rendered as a new take, with its before/after measures.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChainApplied {
    pub added: TakeAdded,
    pub report: ChainReport,
}

/// The result of asking to clean up a take.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum DenoiseOutcome {
    Done {
        added: Box<TakeAdded>,
    },
    /// Model `model` (a manifest id) must be downloaded first.
    NeedsModel {
        model: String,
    },
}

/// `preset`'s settings for `audio` (a source's timeline audio) in `project`. The output gain
/// is then set from a trial render, so the result lands on [`chain::TARGET_LUFS`] (the preset's
/// own estimate can't know how hard the compressor and EQ will work on real material).
pub fn pick(project: &Project, audio: &[f32], preset: ChainPreset) -> ChainPick {
    pick_audio(
        audio,
        project.sample_rate,
        project.analysis.tempo.bpm,
        preset,
    )
}

/// [`pick`] from the audio alone (`sr` its rate, `bpm` the song's tempo): what a compute worker
/// runs without the song.
pub fn pick_audio(audio: &[f32], sr: u32, bpm: f64, preset: ChainPreset) -> ChainPick {
    let measures = chain::measure(audio, sr);
    let mut settings = chain::settings(preset, &measures, bpm);
    let trial = chain::lufs(
        &chain::render(
            audio,
            sr,
            ChainSettings {
                output_db: 0.0,
                ..settings
            },
        ),
        sr,
    );
    if trial > chain::SILENCE_LUFS {
        settings = ChainSettings {
            output_db: chain::TARGET_LUFS - trial,
            ..settings
        }
        .clamped();
    }
    ChainPick { settings, measures }
}

/// `audio` through the chain, and its measures before and after.
pub fn render_chain(
    project: &Project,
    audio: &[f32],
    settings: ChainSettings,
) -> (Vec<f32>, ChainReport) {
    let out = chain::render(audio, project.sample_rate, settings);
    let report = ChainReport {
        before: chain::measure(audio, project.sample_rate),
        after: chain::measure(&out, project.sample_rate),
    };
    (out, report)
}

/// A generation request for `project`, conditioned on its tempo and key.
pub fn request(
    project: &Project,
    task: GenerationTask,
    prompt: &str,
    seed: u32,
) -> GenerationRequest {
    GenerationRequest {
        task,
        prompt: prompt.trim().to_string(),
        seed,
        model: GENERATION_MODEL.to_string(),
        bpm: project.analysis.tempo.bpm,
        key: project
            .analysis
            .keys
            .first()
            .map(|k| k.label.clone())
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::StereoAudio;
    use crate::chain::{lufs, render, TARGET_LUFS};
    use crate::project::{self, SongAudio, SongSource, TrackedPitch};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    const SR: u32 = 44_100;

    #[test]
    fn every_preset_renders_at_the_target_loudness() {
        let tone = |amp: f32| -> Vec<f32> {
            (0..SR as usize * 3)
                .map(|i| {
                    let t = i as f32 / SR as f32;
                    let swell = 0.5 + 0.5 * (std::f32::consts::TAU * 0.7 * t).sin();
                    amp * swell
                        * (1..12)
                            .map(|h| {
                                (std::f32::consts::TAU * 196.0 * h as f32 * t).sin() / h as f32
                            })
                            .sum::<f32>()
                })
                .collect()
        };
        let song = tone(0.3);
        let project = project::build(
            SongSource::File { name: "s".into() },
            &SongAudio::Unseparated {
                mix: StereoAudio {
                    sample_rate: SR,
                    left: song.clone(),
                    right: song.clone(),
                },
            },
            TrackedPitch::yin(&song, SR),
        );
        // (Output gain is capped at ±24 dB, so far quieter takes stop short.)
        for amp in [0.05, 0.6] {
            let take = tone(amp);
            for p in ChainPreset::ALL {
                let pick = pick(&project, &take, p);
                let out = lufs(&render(&take, SR, pick.settings), SR);
                assert!((out - TARGET_LUFS).abs() < 0.5, "{p:?} at {amp}: {out}");
            }
        }
    }
}
