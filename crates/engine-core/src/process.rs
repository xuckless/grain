//! The master take and its processing chain (AGENTS.md §8.7). Every take keeps its raw recording
//! untouched and carries one chain of steps in a fixed order:
//!
//! ```text
//! raw → Pitch edits → Auto-tune → Clean up → Vocal chain → what you hear / export
//! ```
//!
//! Each step is `Unset`, `Active` or `Bypassed` (keeping its parameters). Rendering is cached
//! per step: changing step 3 re-renders steps 3 and 4 only. Scoring always uses the raw take
//! (it measures the singing, not the processing). Hosts run rendering off the UI thread and
//! supply the Clean-up stage (DeepFilterNet is native-only); everything else runs here.

use crate::chain::{self, ChainPreset, ChainSettings};
use crate::edit::{self, EditSource};
use crate::pitch::{PitchFrame, PitchTrack};
use crate::project::Project;
use crate::take::{Take, TakeError};
use crate::tune::{self, TuneSettings};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use ts_rs::TS;

/// One chain step's state. `Bypassed` keeps the parameters so switching it back on restores them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "state", content = "params")]
#[ts(export)]
pub enum Step<T> {
    Unset,
    Active(T),
    Bypassed(T),
}

impl<T> Default for Step<T> {
    fn default() -> Self {
        Self::Unset
    }
}

impl<T> Step<T> {
    /// The parameters while the step is on.
    pub fn active(&self) -> Option<&T> {
        match self {
            Self::Active(p) => Some(p),
            Self::Unset | Self::Bypassed(_) => None,
        }
    }
}

/// The Pitch edits step: the edit list itself lives in `Project::pitch_edits` (one per source).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PitchStep {}

/// The Clean up step: the DeepFilterNet model (a manifest id).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CleanStep {
    pub model: String,
}

/// The Vocal chain step: the preset it started from and its settings.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct VocalStep {
    pub preset: ChainPreset,
    pub settings: ChainSettings,
}

/// A take's processing chain, in its fixed order (AGENTS.md §8.7: a fixed order is the simplest
/// that serves singing; pitch before tuning, clean before colouring).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TakeChain {
    #[serde(default)]
    pub pitch: Step<PitchStep>,
    #[serde(default)]
    pub tune: Step<TuneSettings>,
    #[serde(default)]
    pub clean: Step<CleanStep>,
    #[serde(default)]
    pub vocal: Step<VocalStep>,
}

impl TakeChain {
    /// Whether any step changes the raw take.
    pub fn is_neutral(&self) -> bool {
        self.pitch.active().is_none() && self.tune.active().is_none() && self.clean.active().is_none() && self.vocal.active().is_none()
    }
}

/// One step to run, with what it needs.
#[derive(Debug, Clone, PartialEq)]
pub enum Stage {
    Pitch(edit::PitchEdit),
    Tune {
        settings: TuneSettings,
        /// The pitch the tuner follows: the take's own, or after the pitch edits.
        track: PitchTrack,
    },
    Clean(CleanStep),
    Vocal(VocalStep),
}

impl Stage {
    fn fingerprint(&self, upstream: u64) -> u64 {
        let mut h = DefaultHasher::new();
        upstream.hash(&mut h);
        // Debug formatting covers every parameter (floats included) and is stable in-process,
        // which is all an in-memory cache key needs.
        let tag = match self {
            Self::Pitch(e) => format!("{e:?}"),
            Self::Tune { settings, track } => format!("{settings:?}{}", track.frames.len()),
            Self::Clean(c) => format!("{c:?}"),
            Self::Vocal(v) => format!("{v:?}"),
        };
        std::mem::discriminant(self).hash(&mut h);
        tag.hash(&mut h);
        h.finish()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    #[error(transparent)]
    Take(#[from] TakeError),
    #[error(transparent)]
    Edit(#[from] edit::EditError),
    #[error("{0}")]
    Stage(String),
}

fn take<'a>(project: &'a Project, id: u32) -> Result<&'a Take, ProcessError> {
    project.takes.iter().find(|t| t.id == id).ok_or(TakeError::Unknown(id).into())
}

/// The steps that run for take `id`, in order (bypassed and unset steps are skipped).
pub fn plan(project: &Project, id: u32) -> Result<Vec<Stage>, ProcessError> {
    let t = take(project, id)?;
    let mut out = Vec::new();
    let mut track = t.pitch.clone();
    if t.chain.pitch.active().is_some() {
        let e = edit::edit_of(project, EditSource::Take { id })?;
        track = PitchTrack {
            hop_seconds: t.pitch.hop_seconds,
            frames: edit::edited_pitch(&e, &t.pitch)
                .into_iter()
                .map(|m| m.map_or(PitchFrame::Unvoiced, |midi| PitchFrame::Voiced { midi }))
                .collect(),
        };
        out.push(Stage::Pitch(e));
    }
    if let Some(settings) = t.chain.tune.active() {
        out.push(Stage::Tune { settings: *settings, track });
    }
    if let Some(c) = t.chain.clean.active() {
        out.push(Stage::Clean(c.clone()));
    }
    if let Some(v) = t.chain.vocal.active() {
        out.push(Stage::Vocal(*v));
    }
    Ok(out)
}

/// Runs one core stage (everything but Clean up, which the host supplies).
pub fn run_core_stage(project: &Project, id: u32, stage: &Stage, input: &[f32]) -> Result<Vec<f32>, ProcessError> {
    let t = take(project, id)?;
    match stage {
        Stage::Pitch(e) => Ok(edit::render(e, &t.pitch, input, project.sample_rate)),
        Stage::Tune { settings, track } => Ok(tune::render_track(project, track, input, *settings)),
        Stage::Vocal(v) => Ok(chain::render(input, project.sample_rate, v.settings)),
        Stage::Clean(_) => Err(ProcessError::Stage("clean-up runs in the host".into())),
    }
}

/// Rendered outputs of a take's chain, one per step that ran, keyed by the cumulative
/// fingerprint of every step up to it.
#[derive(Debug, Default, Clone)]
pub struct ChainCache {
    stages: Vec<(u64, Arc<[f32]>)>,
    /// Steps actually rendered (not reused) since creation: for tests and diagnostics.
    pub renders: usize,
}

impl ChainCache {
    /// The processed take: `raw` through `plan`, reusing cached steps up to the first that
    /// changed. `run` renders one stage.
    pub fn render(
        &mut self,
        raw: &Arc<[f32]>,
        plan: &[Stage],
        mut run: impl FnMut(&Stage, &[f32]) -> Result<Vec<f32>, ProcessError>,
    ) -> Result<Arc<[f32]>, ProcessError> {
        let mut upstream = {
            let mut h = DefaultHasher::new();
            raw.len().hash(&mut h);
            (Arc::as_ptr(raw) as *const f32 as usize).hash(&mut h);
            h.finish()
        };
        let mut current = raw.clone();
        for (i, stage) in plan.iter().enumerate() {
            let fp = stage.fingerprint(upstream);
            match self.stages.get(i) {
                Some((cached, out)) if *cached == fp => current = out.clone(),
                _ => {
                    self.stages.truncate(i);
                    let out: Arc<[f32]> = run(stage, &current)?.into();
                    self.renders += 1;
                    self.stages.push((fp, out.clone()));
                    current = out;
                }
            }
            upstream = fp;
        }
        self.stages.truncate(plan.len());
        Ok(current)
    }
}

/// The master take: the one starred, or else the newest sung take.
pub fn master(project: &Project) -> Option<u32> {
    project
        .master_take
        .filter(|id| project.takes.iter().any(|t| t.id == *id && t.source.is_sung()))
        .or_else(|| project.takes.iter().rev().find(|t| t.source.is_sung()).map(|t| t.id))
}

/// Stars take `id` as the master (`None` clears the star: the newest take is used).
pub fn set_master(project: &mut Project, id: Option<u32>) -> Result<(), ProcessError> {
    if let Some(id) = id {
        take(project, id)?;
    }
    project.master_take = id;
    Ok(())
}

/// Replaces take `id`'s chain.
pub fn set_chain(project: &mut Project, id: u32, chain: TakeChain) -> Result<(), ProcessError> {
    let t = project.takes.iter_mut().find(|t| t.id == id).ok_or(TakeError::Unknown(id))?;
    t.chain = chain;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::StereoAudio;
    use crate::chain::ChainPreset;
    use crate::project::{self, SongAudio, SongSource, TrackedPitch};
    use crate::take::{add_take, TakeSource};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    const SR: u32 = 16_000;

    fn song_with_take() -> (Project, u32, Arc<[f32]>) {
        let tone: Vec<f32> = (0..SR as usize * 2)
            .map(|i| 0.3 * (std::f32::consts::TAU * 220.0 * i as f32 / SR as f32).sin())
            .collect();
        let mut p = project::build(
            SongSource::File { name: "s".into() },
            &SongAudio::Unseparated {
                mix: StereoAudio { sample_rate: SR, left: tone.clone(), right: tone.clone() },
            },
            TrackedPitch::yin(&tone, SR),
        );
        let id = add_take(&mut p, TakeSource::Recorded { latency_ms: 0.0 }, &tone, TrackedPitch::yin(&tone, SR));
        (p, id, tone.into())
    }

    fn vocal(preset: ChainPreset) -> VocalStep {
        VocalStep { preset, settings: chain::settings(preset, &chain::measure(&[0.1; 1600], SR), 120.0) }
    }

    #[test]
    fn a_neutral_chain_is_the_raw_take() {
        let (p, id, raw) = song_with_take();
        let mut cache = ChainCache::default();
        let out = cache.render(&raw, &plan(&p, id).unwrap(), |s, i| run_core_stage(&p, id, s, i)).unwrap();
        assert!(Arc::ptr_eq(&out, &raw));
        assert_eq!(cache.renders, 0);
    }

    #[test]
    fn changing_a_later_step_reuses_the_earlier_ones() {
        let (mut p, id, raw) = song_with_take();
        let mut chain = TakeChain {
            tune: Step::Active(TuneSettings::default()),
            vocal: Step::Active(vocal(ChainPreset::Pop)),
            ..TakeChain::default()
        };
        set_chain(&mut p, id, chain.clone()).unwrap();
        let mut cache = ChainCache::default();
        fn go(cache: &mut ChainCache, raw: &Arc<[f32]>, p: &Project, id: u32) -> Arc<[f32]> {
            cache
                .render(raw, &plan(p, id).unwrap(), |s, i| run_core_stage(p, id, s, i))
                .unwrap()
        }
        go(&mut cache, &raw, &p, id);
        assert_eq!(cache.renders, 2);
        // Same chain again: nothing re-renders.
        go(&mut cache, &raw, &p, id);
        assert_eq!(cache.renders, 2);
        // Only the vocal chain (the last step) changes: one render.
        chain.vocal = Step::Active(vocal(ChainPreset::Ballad));
        set_chain(&mut p, id, chain.clone()).unwrap();
        go(&mut cache, &raw, &p, id);
        assert_eq!(cache.renders, 3);
        // Bypassing the tuner re-renders what follows it, from the raw take.
        chain.tune = Step::Bypassed(TuneSettings::default());
        set_chain(&mut p, id, chain).unwrap();
        let out = go(&mut cache, &raw, &p, id);
        assert_eq!(cache.renders, 4);
        assert_eq!(out.len(), raw.len());
    }

    #[test]
    fn steps_run_in_the_fixed_order() {
        let (mut p, id, _) = song_with_take();
        let edit = edit::edit_of(&p, EditSource::Take { id }).unwrap();
        edit::store(&mut p, edit).unwrap();
        set_chain(
            &mut p,
            id,
            TakeChain {
                pitch: Step::Active(PitchStep {}),
                tune: Step::Active(TuneSettings::default()),
                clean: Step::Active(CleanStep { model: "dfn".into() }),
                vocal: Step::Active(vocal(ChainPreset::Natural)),
            },
        )
        .unwrap();
        let kinds: Vec<&str> = plan(&p, id)
            .unwrap()
            .iter()
            .map(|s| match s {
                Stage::Pitch(_) => "pitch",
                Stage::Tune { .. } => "tune",
                Stage::Clean(_) => "clean",
                Stage::Vocal(_) => "vocal",
            })
            .collect();
        assert_eq!(kinds, ["pitch", "tune", "clean", "vocal"]);
    }

    #[test]
    fn the_master_is_the_starred_take_or_the_newest() {
        let (mut p, a, raw) = song_with_take();
        let b = add_take(&mut p, TakeSource::Recorded { latency_ms: 0.0 }, &raw, TrackedPitch::yin(&raw, SR));
        assert_eq!(master(&p), Some(b));
        set_master(&mut p, Some(a)).unwrap();
        assert_eq!(master(&p), Some(a));
        assert!(set_master(&mut p, Some(99)).is_err());
        crate::take::remove_take(&mut p, a).unwrap();
        assert_eq!(master(&p), Some(b));
    }

    #[test]
    fn old_projects_load_with_neutral_chains_and_no_master() {
        let (p, ..) = song_with_take();
        let mut json: serde_json::Value = serde_json::to_value(&p).unwrap();
        json.as_object_mut().unwrap().remove("master_take");
        json["takes"][0].as_object_mut().unwrap().remove("chain");
        let back: Project = serde_json::from_value(json).unwrap();
        assert!(back.takes[0].chain.is_neutral());
        assert_eq!(back.master_take, None);
    }
}
