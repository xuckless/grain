//! Takes' processing chains on desktop (AGENTS.md §8.7): rendering on a background thread
//! (Tauri's blocking pool), Clean up through the native DeepFilterNet, the per-step cache of the
//! take last edited, and the master take. Every change autosaves like every take change.

use crate::{fail, with_loaded, Engine};
use engine_core::process::{self, ChainCache, ProcessError, Stage, TakeChain};
use engine_core::project::Project;
use engine_native::denoise::{self, Denoiser};
use serde::Serialize;
use std::sync::Arc;
use tauri::AppHandle;

/// The per-step render cache of the one take last edited (kept for one take only, so memory
/// stays at a few copies of one take whatever the project holds).
pub struct Rendered {
    pub take: u32,
    pub cache: ChainCache,
}

/// What a chain change did.
/// Mirrored by hand as `ChainOutcome` in packages/engine-api.
#[derive(Serialize)]
#[serde(tag = "kind")]
pub enum ChainOutcome {
    Rendered {
        project: Box<Project>,
    },
    /// The Clean up step needs this model downloaded first; nothing changed.
    NeedsModel {
        model: String,
    },
}

/// Take `id`'s processed audio: its raw timeline audio through its chain, reusing `cache`.
pub fn render(
    engine: &Engine,
    project: &Project,
    id: u32,
    raw: &Arc<[f32]>,
    cache: &mut ChainCache,
) -> anyhow::Result<Arc<[f32]>> {
    let plan = process::plan(project, id)?;
    let mut denoiser = match plan.iter().any(|s| matches!(s, Stage::Clean(_))) {
        true => Some(Denoiser::load(
            &engine
                .models
                .lock()
                .expect("models lock")
                .find(denoise::ARCHIVE)
                .ok_or_else(|| anyhow::anyhow!("the clean-up model isn't downloaded"))?,
        )?),
        false => None,
    };
    let out = cache.render(id, raw, &plan, |stage, input| {
        match (stage, &mut denoiser) {
            (Stage::Clean(_), Some(d)) => d
                .run(input, project.sample_rate)
                .map_err(|e| ProcessError::Stage(e.to_string())),
            _ => process::run_core_stage(project, id, stage, input),
        }
    })?;
    Ok(out)
}

/// Every take's player audio for a session start: raw for a neutral chain, else processed. A
/// take whose chain can't render (its model was deleted) plays raw, with a warning.
pub fn player_takes(engine: &Engine, project: &Project, raw: Vec<Vec<f32>>) -> Vec<Vec<f32>> {
    project
        .takes
        .iter()
        .zip(raw)
        .map(|(t, audio)| {
            if t.chain.is_neutral() {
                return audio;
            }
            let raw: Arc<[f32]> = audio.into();
            match render(engine, project, t.id, &raw, &mut ChainCache::default()) {
                Ok(out) => out.to_vec(),
                Err(e) => {
                    eprintln!("take {} plays unprocessed: {e}", t.id);
                    raw.to_vec()
                }
            }
        })
        .collect()
}

/// Replaces take `id`'s chain, renders it off the UI thread (the session lock is held only to
/// read and to swap in the result) and saves.
#[tauri::command]
pub async fn set_take_chain(
    app: AppHandle,
    id: u32,
    chain: TakeChain,
) -> Result<ChainOutcome, String> {
    crate::produce::blocking(app, move |engine| {
        if chain.clean.active().is_some()
            && engine
                .models
                .lock()
                .expect("models lock")
                .find(denoise::ARCHIVE)
                .is_none()
        {
            return Ok(ChainOutcome::NeedsModel {
                model: denoise::MODEL.to_string(),
            });
        }
        let (key, mut project, mut cache) = with_loaded(engine, |l| {
            let cache = match l.rendered.take() {
                Some(r) if r.take == id => r.cache,
                _ => ChainCache::default(),
            };
            Ok((l.cache_key.clone(), l.project.clone(), cache))
        })?;
        process::set_chain(&mut project, id, chain.clone())?;
        let raw: Arc<[f32]> = engine.cache.take_audio(&key, id)?.into();
        let out = render(engine, &project, id, &raw, &mut cache)?;
        with_loaded(engine, |l| {
            process::set_chain(&mut l.project, id, chain)?;
            let index = engine_core::take::player_index(&l.project, id)
                .ok_or_else(|| anyhow::anyhow!("take {id} was deleted"))?;
            l.playback.replace_take(index as usize, &out)?;
            l.rendered = Some(Rendered { take: id, cache });
            engine.cache.save_project(&l.cache_key, &l.project)?;
            Ok(ChainOutcome::Rendered {
                project: Box::new(l.project.clone()),
            })
        })
    })
    .await
}

/// Stars take `id` as the master (`None` clears it); saved at once.
#[tauri::command(async)]
pub fn set_master(engine: tauri::State<Engine>, id: Option<u32>) -> Result<Project, String> {
    with_loaded(&engine, |l| {
        process::set_master(&mut l.project, id)?;
        engine.cache.save_project(&l.cache_key, &l.project)?;
        Ok(l.project.clone())
    })
    .map_err(fail)
}
