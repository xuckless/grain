//! Tauri desktop host: exposes engine-native to the shared Svelte UI.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use engine_core::player::{PlayerCommand, Transport, TransportSnapshot};
use engine_core::project::{
    self, ImportStage, Project, ProjectSummary, SeparationQuality, SongAudio, SongSource,
};
use engine_core::score::{ScoreReport, ScoreSettings};
use engine_core::WORKING_SAMPLE_RATE;
use engine_native::cache::{Lookup, StemCache};
use engine_native::decode;
use engine_native::import::{import_song, Imported, Separation};
use engine_native::pitch::Pitch;
use engine_native::playback::Playback;
use engine_native::separate::Separators;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, State};

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
    Loaded(Box<Separators>),
    /// No weights found; songs import as a single mix track.
    Missing,
}

struct Engine {
    session: Mutex<Session>,
    model: Mutex<Model>,
    pitch: Mutex<Pitch>,
    cache: StemCache,
    model_dirs: Vec<PathBuf>,
    #[cfg(feature = "youtube")]
    downloads: PathBuf,
}

/// Errors cross IPC as strings.
fn fail(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn import(
    app: &AppHandle,
    path: &Path,
    song: SongSource,
    quality: SeparationQuality,
) -> anyhow::Result<Project> {
    let engine = app.state::<Engine>();
    let mut model = engine.model.lock().expect("model lock");
    if let Model::NotLoaded = *model {
        *model = match Separators::find(&engine.model_dirs)? {
            Some(m) => Model::Loaded(Box::new(m)),
            None => Model::Missing,
        };
    }
    let separation = match &mut *model {
        Model::Loaded(separators) => Separation::Model {
            separators,
            quality,
        },
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
        &mut engine.pitch.lock().expect("pitch lock"),
        &engine.cache,
        |stage: ImportStage| {
            let _ = app.emit("import-stage", stage);
        },
    )?;
    drop(model);
    start_session(&engine, cache_key, project, &audio)
}

/// Makes `project` the loaded song and starts its playback stream.
fn start_session(
    engine: &Engine,
    cache_key: String,
    project: Project,
    audio: &SongAudio,
) -> anyhow::Result<Project> {
    let mut session = engine.session.lock().expect("session lock");
    // Close the old stream before opening a new one.
    *session = Session::Empty;
    let playback = Playback::start(&project, audio)?;
    *session = Session::Loaded {
        cache_key,
        project: Box::new(project.clone()),
        playback,
    };
    Ok(project)
}

/// Saved projects (stem-cache entries), most recent first.
#[tauri::command]
async fn list_projects(app: AppHandle) -> Result<Vec<ProjectSummary>, String> {
    tauri::async_runtime::spawn_blocking(move || app.state::<Engine>().cache.list())
        .await
        .map_err(fail)?
        .map_err(fail)
}

#[tauri::command]
async fn open_project(app: AppHandle, id: String) -> Result<Project, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let engine = app.state::<Engine>();
        match engine.cache.lookup(&id)? {
            Lookup::Hit { project, audio } => start_session(&engine, id, *project, &audio),
            Lookup::Miss => anyhow::bail!("this project is no longer on disk"),
        }
    })
    .await
    .map_err(fail)?
    .map_err(fail)
}

/// Writes the loaded song's mix settings to its project.
#[tauri::command]
fn save_project(engine: State<Engine>) -> Result<(), String> {
    match &*engine.session.lock().map_err(fail)? {
        Session::Loaded {
            cache_key, project, ..
        } => engine.cache.save_project(cache_key, project).map_err(fail),
        Session::Empty => Err("no song loaded".into()),
    }
}

/// Stops playback and unloads the song.
#[tauri::command]
fn close_session(engine: State<Engine>) -> Result<(), String> {
    *engine.session.lock().map_err(fail)? = Session::Empty;
    Ok(())
}

#[tauri::command]
async fn import_file(
    app: AppHandle,
    path: String,
    quality: SeparationQuality,
) -> Result<Project, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let path = PathBuf::from(path);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        import(&app, &path, SongSource::File { name }, quality)
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

/// Which separations the UI may offer (AGENTS.md §4.2).
#[derive(Serialize)]
#[serde(tag = "kind")]
enum SeparationOffer {
    #[serde(rename = "choice")]
    Choice,
    #[serde(rename = "fastOnly")]
    FastOnly { reason: String },
}

#[tauri::command]
fn separation_offer(engine: State<Engine>) -> SeparationOffer {
    if Separators::offers(&engine.model_dirs, SeparationQuality::BestVocals) {
        SeparationOffer::Choice
    } else {
        SeparationOffer::FastOnly {
            reason: "The vocal model isn't installed (scripts/fetch_models.py).".into(),
        }
    }
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
async fn import_url(
    app: AppHandle,
    url: String,
    quality: SeparationQuality,
) -> Result<Project, String> {
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
            quality,
        )
    })
    .await
    .map_err(fail)?
    .map_err(fail)
}

#[cfg(not(feature = "youtube"))]
#[tauri::command]
async fn import_url(_url: String, _quality: SeparationQuality) -> Result<Project, String> {
    Err("this build has no YouTube import".into())
}

#[tauri::command]
fn player(engine: State<Engine>, command: PlayerCommand) -> Result<(), String> {
    let mut session = engine.session.lock().map_err(fail)?;
    let Session::Loaded {
        project, playback, ..
    } = &mut *session
    else {
        return Err("no song loaded".into());
    };
    playback.send(command).map_err(fail)?;
    if let PlayerCommand::SetMix { track, params } = command {
        // Mix settings are part of the project; `save_project` persists them.
        if let Some(t) = project.tracks.get_mut(track as usize) {
            t.mix = params;
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
        let engine = app.state::<Engine>();
        let project = match &*engine.session.lock().map_err(fail)? {
            Session::Loaded { project, .. } => project.clone(),
            Session::Empty => return Err("load a song first".to_string()),
        };
        let take = decode::load(Path::new(&path), project.sample_rate).map_err(fail)?;
        let take = engine
            .pitch
            .lock()
            .map_err(fail)?
            .track(
                project.analysis.pitch_tracker,
                &take.to_mono(),
                project.sample_rate,
            )
            .map_err(fail)?;
        project::score_take(&project, &take, settings).map_err(fail)
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
                pitch: Mutex::new(Pitch::find(&model_dirs)),
                cache: StemCache::new(data.join("stems")),
                model_dirs,
                #[cfg(feature = "youtube")]
                downloads: data.join("downloads"),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            import_file,
            separation_offer,
            url_import,
            import_url,
            player,
            transport,
            score_take,
            list_projects,
            open_project,
            save_project,
            close_session
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
