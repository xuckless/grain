//! Produce on desktop (AGENTS.md §8): the vocal chain, harmony and melody MIDI, DeepFilterNet
//! clean-up and ACE-Step generation. Each result is added as a new take of the loaded song.

use super::{add_take, edit_audio, fail, track_timeline, with_loaded, Engine};
use anyhow::Context;
use engine_core::chain::ChainPreset;
use engine_core::edit::EditSource;
use engine_core::generate::{self, GenerationTask, GeneratorEvent, GeneratorState};
use engine_core::harmony::{self, Interval};
use engine_core::produce::{self, ChainPick};
use engine_core::project::Project;
use engine_core::score::ScoreSettings;
use engine_core::take::{TakeAdded, TakeSource};
use engine_native::generate::{find_uv, Install};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager, State};

/// The generation sidecar's install and where it is in its lifecycle.
pub struct Generator {
    pub state: GeneratorState,
    /// Install root (app data).
    pub root: PathBuf,
    /// The service's source: bundled with the app, or the repository's in development.
    pub service: PathBuf,
}

impl Generator {
    pub fn new(root: PathBuf, resources: Option<PathBuf>) -> Self {
        let bundled = resources.map(|r| r.join("ml-sidecar"));
        let service = bundled.filter(|p| p.is_dir()).unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../services/ml-sidecar")
        });
        let installed = root.join("installed").is_file();
        Self {
            state: if installed {
                GeneratorState::Ready
            } else {
                GeneratorState::NotInstalled
            },
            root,
            service,
        }
    }

    fn install(&self) -> anyhow::Result<Install> {
        Ok(Install {
            root: self.root.clone(),
            uv: find_uv().context(
                "uv isn't installed: get it from docs.astral.sh/uv (brew install uv), then retry",
            )?,
        })
    }
}

/// Applies `event` to the generator's state (rejecting what the state machine rejects) and
/// tells the UI.
fn advance(app: &AppHandle, event: GeneratorEvent) -> anyhow::Result<GeneratorState> {
    let engine = app.state::<Engine>();
    let mut g = engine.generator.lock().expect("generator lock");
    let next = generate::next(&g.state, event.clone())
        .with_context(|| format!("can't {event:?} while {:?}", g.state))?;
    g.state = next.clone();
    let _ = app.emit("generator-state", next.clone());
    Ok(next)
}

/// The loaded song's cache key, project and `source`'s timeline audio.
fn source(engine: &Engine, source: EditSource) -> anyhow::Result<(Project, Vec<f32>)> {
    let (key, project) = with_loaded(engine, |l| Ok((l.cache_key.clone(), l.project.clone())))?;
    let audio = edit_audio(engine, &key, &project, source)?;
    Ok((project, audio))
}

/// Tracks `audio`'s pitch and adds it as a take from `from`.
fn add(
    engine: &Engine,
    project: &Project,
    from: TakeSource,
    audio: &[f32],
    settings: ScoreSettings,
) -> anyhow::Result<TakeAdded> {
    let pitch = track_timeline(engine, project, audio)?;
    add_take(engine, from, audio, pitch, settings)
}

pub(crate) async fn blocking<T: Send + 'static>(
    app: AppHandle,
    f: impl FnOnce(&Engine) -> anyhow::Result<T> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || f(&app.state::<Engine>()))
        .await
        .map_err(fail)?
        .map_err(fail)
}

/// Measures a source and gives `preset`'s settings for it.
#[tauri::command]
pub async fn chain_pick(
    app: AppHandle,
    from: EditSource,
    preset: ChainPreset,
) -> Result<ChainPick, String> {
    blocking(app, move |engine| {
        let (project, audio) = source(engine, from)?;
        Ok(produce::pick(&project, &audio, preset))
    })
    .await
}

/// Sings a harmony line from a source's own voice as a new track.
#[tauri::command]
pub async fn harmony(
    app: AppHandle,
    from: EditSource,
    interval: Interval,
    settings: ScoreSettings,
) -> Result<TakeAdded, String> {
    blocking(app, move |engine| {
        let (project, audio) = source(engine, from)?;
        let out = harmony::render(&project, from, &audio, interval)?;
        add(
            engine,
            &project,
            TakeSource::Harmony { from, interval },
            &out,
            settings,
        )
    })
    .await
}

/// Writes a source's melody (or its harmony at `interval`) to `path` as a MIDI file.
#[tauri::command(async)]
pub fn export_melody(
    engine: State<Engine>,
    from: EditSource,
    interval: Option<Interval>,
    path: PathBuf,
) -> Result<(), String> {
    let bytes = with_loaded(&engine, |l| {
        Ok(harmony::melody_midi(&l.project, from, interval)?)
    })
    .map_err(fail)?;
    std::fs::write(path, bytes).map_err(fail)
}

#[tauri::command(async)]
pub fn generator_state(engine: State<Engine>) -> GeneratorState {
    engine
        .generator
        .lock()
        .expect("generator lock")
        .state
        .clone()
}

/// Installs the generation sidecar (Python environment and ACE-Step weights). Long; states
/// arrive as "generator-state".
#[tauri::command]
pub async fn generator_install(app: AppHandle) -> Result<GeneratorState, String> {
    advance(&app, GeneratorEvent::Install).map_err(fail)?;
    let worker = app.clone();
    let result = blocking(app.clone(), move |engine| {
        let (install, service) = {
            let g = engine.generator.lock().expect("generator lock");
            (g.install()?, g.service.clone())
        };
        install.install(&service, |step| {
            let _ = advance(
                &worker,
                GeneratorEvent::Step {
                    step: step.to_string(),
                },
            );
        })
    })
    .await;
    let event = match result {
        Ok(()) => GeneratorEvent::Installed,
        Err(message) => GeneratorEvent::Fail { message },
    };
    advance(&app, event).map_err(fail)
}

/// Removes the sidecar's environment and weights.
#[tauri::command]
pub async fn generator_uninstall(app: AppHandle) -> Result<GeneratorState, String> {
    let root = app
        .state::<Engine>()
        .generator
        .lock()
        .expect("generator lock")
        .root
        .clone();
    blocking(app.clone(), move |_| {
        engine_native::generate::uninstall(&root)
    })
    .await?;
    advance(&app, GeneratorEvent::Uninstalled).map_err(fail)
}

/// The audio a task is conditioned on: a vocal source, or the song's whole mix.
fn conditioning(engine: &Engine, task: GenerationTask) -> anyhow::Result<(Project, Vec<f32>)> {
    match task {
        GenerationTask::Backing { from } | GenerationTask::Lego { from, .. } => {
            source(engine, from)
        }
        GenerationTask::Repaint { .. } => {
            let (key, project) =
                with_loaded(engine, |l| Ok((l.cache_key.clone(), l.project.clone())))?;
            let mut mix = vec![0.0f32; project.frames as usize];
            for t in &project.tracks {
                for (m, v) in mix
                    .iter_mut()
                    .zip(engine.cache.track_audio(&key, t.source)?)
                {
                    *m += v;
                }
            }
            Ok((project, mix))
        }
    }
}

/// Runs one ACE-Step generation in the sidecar (started for it, stopped after) and adds the
/// result as a new track that keeps its model, prompt and seed.
#[tauri::command]
pub async fn generate(
    app: AppHandle,
    task: GenerationTask,
    prompt: String,
    seed: u32,
    settings: ScoreSettings,
) -> Result<TakeAdded, String> {
    advance(&app, GeneratorEvent::Start).map_err(fail)?;
    let result = blocking(app.clone(), move |engine| {
        let install = engine.generator.lock().expect("generator lock").install()?;
        let (project, audio) = conditioning(engine, task)?;
        let request = produce::request(&project, task, &prompt, seed);
        let sidecar = install.start()?;
        let out = sidecar.generate(&request, &audio, project.sample_rate)?;
        drop(sidecar);
        add(
            engine,
            &project,
            TakeSource::Generated { request },
            &out,
            settings,
        )
    })
    .await;
    let event = match &result {
        Ok(_) => GeneratorEvent::Finished,
        Err(message) => GeneratorEvent::Fail {
            message: message.clone(),
        },
    };
    advance(&app, event).map_err(fail)?;
    result
}
