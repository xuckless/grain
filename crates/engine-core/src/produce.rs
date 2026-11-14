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

/// `preset`'s settings for `audio` (a source's timeline audio) in `project`.
pub fn pick(project: &Project, audio: &[f32], preset: ChainPreset) -> ChainPick {
    let measures = chain::measure(audio, project.sample_rate);
    ChainPick {
        settings: chain::settings(preset, &measures, project.analysis.tempo.bpm),
        measures,
    }
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
