//! Tauri desktop host: exposes engine-native to the shared Svelte UI.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use engine_core::player::{PlayerCommand, Transport, TransportSnapshot};
use engine_core::project::{self, ImportStage, Project, SongSource};
use engine_core::score::{ScoreReport, ScoreSettings};
use engine_core::WORKING_SAMPLE_RATE;
use engine_native::cache::StemCache;
use engine_native::decode;
use engine_native::import::{import_song, Imported, Separation};
use engine_native::playback::Playback;
use engine_native::separate::Demucs;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

const MODEL_FILE: &str = "htdemucs_6s.onnx";

/// The loaded song, if any.
enum Session {
    Empty,
    Loaded {
        cache_key: String,
        project: Box<Project>,
        playback: Playback,
    },
}

/// The separation model, loaded on first import.
enum Model {
    NotLoaded,
    Loaded(Demucs),
    /// No weights found; songs import as a single mix track.
    Missing,
}

struct Engine {
    session: Mutex<Session>,
    model: Mutex<Model>,
    cache: StemCache,
    model_dirs: Vec<PathBuf>,
    #[cfg(feature = "youtube")]
    downloads: PathBuf,
}

/// Errors cross IPC as strings.
fn fail(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn import(app: &AppHandle, path: &Path, song: SongSource) -> anyhow::Result<Project> {
    let engine = app.state::<Engine>();
    let mut model = engine.model.lock().expect("model lock");
    if let Model::NotLoaded = *model {
        *model = match engine
            .model_dirs
            .iter()
            .map(|d| d.join(MODEL_FILE))
            .find(|p| p.exists())
        {
            Some(path) => Model::Loaded(Demucs::load(&path)?),
            None => Model::Missing,
        };
    }
    let separation = match &mut *model {
        Model::Loaded(m) => Separation::Model(m),
        Model::Missing | Model::NotLoaded => Separation::Disabled,
    };
    let Imported {
        cache_key,
        project,
        audio,
    } = import_song(
        path,
        song,
        separation,
        &engine.cache,
        |stage: ImportStage| {
            let _ = app.emit("import-stage", stage);
        },
    )?;
    drop(model);
    let mut session = engine.session.lock().expect("session lock");
    // Close the old stream before opening a new one.
    *session = Session::Empty;
    let playback = Playback::start(&project, &audio)?;
    *session = Session::Loaded {
        cache_key,
        project: Box::new(project.clone()),
        playback,
    };
    Ok(project)
}

#[tauri::command]
async fn import_file(app: AppHandle, path: String) -> Result<Project, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = PathBuf::from(path);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        import(&app, &path, SongSource::File { name })
    })
    .await
    .map_err(fail)?
    .map_err(fail)
}

#[derive(Serialize)]
#[serde(tag = "kind")]
enum UrlImport {
    #[serde(rename = "available")]
    Available,
    #[serde(rename = "unavailable")]
    Unavailable,
}

#[tauri::command]
fn url_import() -> UrlImport {
    if cfg!(feature = "youtube") {
        UrlImport::Available
    } else {
        UrlImport::Unavailable
    }
}

#[cfg(feature = "youtube")]
#[tauri::command]
async fn import_url(app: AppHandle, url: String) -> Result<Project, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let downloads = app.state::<Engine>().downloads.clone();
        std::fs::create_dir_all(&downloads)?;
        let d = ingest::download(&ingest::Tools::from_path(), &url, &downloads)?;
        import(
            &app,
            &d.wav,
            SongSource::YouTube {
                url: d.url,
                title: d.title,
            },
        )
    })
    .await
    .map_err(fail)?
    .map_err(fail)
}

#[cfg(not(feature = "youtube"))]
#[tauri::command]
async fn import_url(_url: String) -> Result<Project, String> {
    Err("this build has no YouTube import".into())
}

#[tauri::command]
fn player(engine: State<Engine>, command: PlayerCommand) -> Result<(), String> {
    let mut session = engine.session.lock().map_err(fail)?;
    let Session::Loaded {
        cache_key,
        project,
        playback,
    } = &mut *session
    else {
        return Err("no song loaded".into());
    };
    playback.send(command).map_err(fail)?;
    if let PlayerCommand::SetMix { track, params } = command {
        // Mix settings are part of the saved project.
        if let Some(t) = project.tracks.get_mut(track as usize) {
            t.mix = params;
            engine
                .cache
                .save_project(cache_key, project)
                .map_err(fail)?;
        }
    }
    Ok(())
}

#[tauri::command]
fn transport(engine: State<Engine>) -> Result<TransportSnapshot, String> {
    let session = engine.session.lock().map_err(fail)?;
    match &*session {
        Session::Loaded { playback, .. } => Ok(TransportSnapshot {
            transport: playback.transport(),
            sample_rate: playback.sample_rate(),
        }),
        Session::Empty => Ok(TransportSnapshot {
            transport: Transport::Stopped,
            sample_rate: WORKING_SAMPLE_RATE,
        }),
    }
}

#[tauri::command]
async fn score_take(
    app: AppHandle,
    path: String,
    settings: ScoreSettings,
) -> Result<ScoreReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let project = match &*app.state::<Engine>().session.lock().map_err(fail)? {
            Session::Loaded { project, .. } => project.clone(),
            Session::Empty => return Err("load a song first".to_string()),
        };
        let take = decode::load(Path::new(&path), project.sample_rate).map_err(fail)?;
        Ok(project::score_take(&project, &take, settings))
    })
    .await
    .map_err(fail)?
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            let mut model_dirs = vec![data.join("models")];
            // Development builds also look in the repository's models/weights.
            if cfg!(debug_assertions) {
                model_dirs
                    .push(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../models/weights"));
            }
            app.manage(Engine {
                session: Mutex::new(Session::Empty),
                model: Mutex::new(Model::NotLoaded),
                cache: StemCache::new(data.join("stems")),
                model_dirs,
                #[cfg(feature = "youtube")]
                downloads: data.join("downloads"),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            import_file,
            url_import,
            import_url,
            player,
            transport,
            score_take
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
