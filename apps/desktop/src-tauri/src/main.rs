//! Tauri desktop host: exposes engine-native to the shared Svelte UI.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod chain;
mod export;
#[cfg(feature = "youtube")]
mod extractor;
mod produce;

use anyhow::Context;
use engine_core::cue::{self, Latency, LatencySource, PROBE_TAIL_SECONDS};
use engine_core::edit::{self, EditOp, EditSource, EditUpdate, PitchEdit, PitchEditView};
use engine_core::input::InputState;
use engine_core::live::{LiveSession, LiveUpdate};
use engine_core::lyrics::{self, LyricsUpdate, ProjectLyrics};
use engine_core::midi::{self, TranscribeOutcome};
use engine_core::player::{
    MonitorMode, MonitorStatus, PlayerCommand, Transport, TransportSnapshot,
};
use engine_core::project::{
    self, ImportStage, Project, ProjectSummary, SeparationQuality, SongAudio, SongSource,
    TrackSource, TrackedPitch,
};
use engine_core::score::{ScoreReport, ScoreSettings};
use engine_core::take::{self, TakeAdded, TakeSource};
use engine_core::tune::{self, TunePreview, TuneSettings};
use engine_core::WORKING_SAMPLE_RATE;
use engine_ml::basic_pitch;
use engine_ml::models::{self, Distribution, ModelEntry, ModelState};
use engine_native::cache::{Lookup, StemCache};
use engine_native::capture::{self, Capture, MonitorInput, StreamInstant};
use engine_native::decode;
use engine_native::import::{import_song, Imported, Separation};
use engine_native::latency::{self, LatencyStore};
use engine_native::models::ModelStore;
use engine_native::pitch::Pitch;
use engine_native::playback::Playback;
use engine_native::separate::Separators;
use engine_native::transcribe::Missing;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};

/// The loaded song, if any.
enum Session {
    Empty,
    Loaded(Box<Loaded>),
}

struct Loaded {
    cache_key: String,
    project: Project,
    playback: Playback,
    mic: Mic,
    monitor: Monitor,
    /// The chain render cache of the take last edited (see [`chain::Rendered`]).
    rendered: Option<chain::Rendered>,
    /// Takes deleted or retaken over this session, kept (audio on disk too) so ⌘Z can bring
    /// them back; their audio is removed when the song closes.
    aside: Vec<take::SetAside>,
}

/// The microphone held open for live monitoring (AGENTS.md §7 step 3); the player knows
/// whether the voice is heard dry or tuned.
enum Monitor {
    Off,
    /// The input stream stays open while this is held.
    Listening {
        _input: MonitorInput,
    },
}

/// What the microphone is doing.
enum Mic {
    Closed,
    /// Recording a take. The count-in started at `from_seconds` on the song timeline.
    Recording {
        capture: Capture,
        from_seconds: f64,
        count_in_seconds: f64,
        live: Live,
    },
    /// A latency calibration owns the microphone (its capture lives in that command).
    Calibrating,
}

/// Live karaoke pitch of the take being recorded (AGENTS.md §6.3).
enum Live {
    /// Until the count-in cue and the first input block have both arrived, the capture can't be
    /// placed on the song timeline yet.
    Waiting,
    /// `read`: capture samples already fed to the session.
    Running {
        session: Box<LiveSession>,
        read: usize,
    },
}

/// Capture time (seconds) of song time 0: the cue's position in the capture, then the count-in,
/// plus the round-trip latency, minus where on the song the count-in started.
fn song_zero(
    cue: StreamInstant,
    first: StreamInstant,
    count_in_seconds: f64,
    latency: &Latency,
    from_seconds: f64,
) -> f64 {
    capture::seconds_between(cue, first) + count_in_seconds + f64::from(latency.ms) / 1000.0
        - from_seconds
}

/// Longest wait for the calibration chirp to finish playing.
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// Longest wait for the audio callback to pick up a cue.
const CUE_TIMEOUT: Duration = Duration::from_millis(500);
const POLL: Duration = Duration::from_millis(20);

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
    latency: LatencyStore,
    /// Downloaded models (app data) and, in development builds, the repository's weights.
    models: Mutex<ModelStore>,
    /// The optional AI-generation sidecar (Produce → Generate).
    generator: Mutex<produce::Generator>,
    #[cfg(feature = "youtube")]
    extractor: extractor::Extractor,
}

impl Engine {
    /// Directories models are looked up in.
    fn model_dirs(&self) -> Vec<PathBuf> {
        self.models.lock().expect("models lock").dirs().to_vec()
    }
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
    let dirs = engine.model_dirs();
    let mut model = engine.model.lock().expect("model lock");
    if let Model::NotLoaded = *model {
        *model = match Separators::find(&dirs)? {
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
        takes,
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
    start_session(&engine, cache_key, project, &audio, takes)
}

/// Makes `project` the loaded song and starts its playback stream.
fn start_session(
    engine: &Engine,
    cache_key: String,
    project: Project,
    audio: &SongAudio,
    takes: Vec<Vec<f32>>,
) -> anyhow::Result<Project> {
    // Takes play through their processing chains (AGENTS.md §8.7), rendered before locking.
    let takes = chain::player_takes(engine, &project, takes);
    let mut session = engine.session.lock().expect("session lock");
    // Close the old stream before opening a new one. Rebuilding the same song's player keeps
    // its set-aside takes; any other song's are gone for good.
    let aside = match std::mem::replace(&mut *session, Session::Empty) {
        Session::Loaded(old) if old.cache_key == cache_key => old.aside,
        Session::Loaded(old) => {
            discard_aside(engine, &old);
            Vec::new()
        }
        Session::Empty => Vec::new(),
    };
    let playback = Playback::start(&project, audio, &takes)?;
    *session = Session::Loaded(Box::new(Loaded {
        cache_key,
        project: project.clone(),
        playback,
        mic: Mic::Closed,
        monitor: Monitor::Off,
        rendered: None,
        aside,
    }));
    Ok(project)
}

/// Removes the audio of the takes `loaded` set aside (the song is closing, so no undo needs
/// them). Failures leave an unlisted file behind, which nothing reads.
fn discard_aside(engine: &Engine, loaded: &Loaded) {
    for a in &loaded.aside {
        if let Err(e) = engine.cache.remove_take(&loaded.cache_key, a.take.id) {
            eprintln!("couldn't remove take {}: {e:#}", a.take.id);
        }
    }
}

/// Runs `f` on the loaded song.
fn with_loaded<T>(
    engine: &Engine,
    f: impl FnOnce(&mut Loaded) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    match &mut *engine.session.lock().expect("session lock") {
        Session::Loaded(loaded) => f(loaded),
        Session::Empty => anyhow::bail!("load a song first"),
    }
}

/// Tracks timeline audio with the loaded song's reference tracker.
fn track_timeline(
    engine: &Engine,
    project: &Project,
    timeline: &[f32],
) -> anyhow::Result<TrackedPitch> {
    engine.pitch.lock().expect("pitch lock").track(
        project.analysis.pitch_tracker,
        timeline,
        project.sample_rate,
    )
}

/// Adds a take's timeline audio and pitch to the loaded song: project, cache, player.
/// Returns its score.
fn add_take(
    engine: &Engine,
    source: TakeSource,
    timeline: &[f32],
    pitch: TrackedPitch,
    settings: ScoreSettings,
) -> anyhow::Result<TakeAdded> {
    with_loaded(engine, |l| {
        let id = take::add_take(&mut l.project, source, timeline, pitch);
        engine
            .cache
            .store_take(&l.cache_key, id, timeline, l.project.sample_rate)?;
        let added = l.project.takes.last().expect("just added");
        l.playback.add_take(added, timeline)?;
        // Takes are saved the moment they exist (AGENTS.md §11, autosave).
        engine.cache.save_project(&l.cache_key, &l.project)?;
        Ok(TakeAdded {
            report: take::score_take(&l.project, id, settings)?,
            project: l.project.clone(),
            take: id,
        })
    })
}

fn device_pair(playback: &Playback) -> String {
    latency::devices(&capture::default_input_name(), playback.device())
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
            Lookup::Hit {
                project,
                audio,
                takes,
            } => start_session(&engine, id, *project, &audio, takes),
            Lookup::Miss => anyhow::bail!("this project is no longer on disk"),
        }
    })
    .await
    .map_err(fail)?
    .map_err(fail)
}

/// Writes the loaded song's mix settings to its project.
#[tauri::command(async)]
fn save_project(engine: State<Engine>) -> Result<(), String> {
    with_loaded(&engine, |l| {
        engine.cache.save_project(&l.cache_key, &l.project)
    })
    .map_err(fail)
}

/// Renames a take; saved at once like every take change. Returns the project.
#[tauri::command(async)]
fn rename_take(engine: State<Engine>, id: u32, name: String) -> Result<Project, String> {
    with_loaded(&engine, |l| {
        take::rename_take(&mut l.project, id, &name)?;
        engine.cache.save_project(&l.cache_key, &l.project)?;
        Ok(l.project.clone())
    })
    .map_err(fail)
}

/// Changes which takes the song holds (`f` sets takes aside or puts them back), saves at once,
/// and rebuilds the player (its tracks are indexed by position, so playback stops). Returns the
/// project.
async fn rearrange_takes(
    app: AppHandle,
    f: impl FnOnce(&mut Project, &mut Vec<take::SetAside>) -> Result<(), take::TakeError>
        + Send
        + 'static,
) -> Result<Project, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let engine = app.state::<Engine>();
        let key = with_loaded(&engine, |l| {
            f(&mut l.project, &mut l.aside)?;
            engine.cache.save_project(&l.cache_key, &l.project)?;
            Ok(l.cache_key.clone())
        })?;
        match engine.cache.lookup(&key)? {
            Lookup::Hit {
                project,
                audio,
                takes,
            } => start_session(&engine, key, *project, &audio, takes),
            Lookup::Miss => anyhow::bail!("this project is no longer on disk"),
        }
    })
    .await
    .map_err(fail)?
    .map_err(fail)
}

/// Deletes a take, saved at once. Its audio is kept until the song closes, so ⌘Z restores it.
#[tauri::command]
async fn delete_take(app: AppHandle, id: u32) -> Result<Project, String> {
    rearrange_takes(app, move |p, aside| take::set_aside(p, aside, id)).await
}

/// Puts a deleted take back where it was.
#[tauri::command]
async fn restore_take(app: AppHandle, id: u32) -> Result<Project, String> {
    rearrange_takes(app, move |p, aside| take::put_back(p, aside, id)).await
}

/// Take `replacement` takes over take `target`'s place (a retake, or undoing or redoing one).
#[tauri::command]
async fn replace_take(app: AppHandle, target: u32, replacement: u32) -> Result<Project, String> {
    rearrange_takes(app, move |p, aside| {
        take::replace_take(p, aside, target, replacement)
    })
    .await
}

/// Stops playback and unloads the song.
#[tauri::command(async)]
fn close_session(engine: State<Engine>) -> Result<(), String> {
    let old = std::mem::replace(&mut *engine.session.lock().map_err(fail)?, Session::Empty);
    if let Session::Loaded(old) = old {
        discard_aside(&engine, &old);
    }
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

/// Whether YouTube import works, and with which yt-dlp (AGENTS.md §4.1).
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum UrlImport {
    #[cfg(feature = "youtube")]
    Available { origin: extractor::Origin },
    #[cfg(feature = "youtube")]
    Missing { reason: String },
    /// This build has no YouTube import.
    #[cfg_attr(feature = "youtube", allow(dead_code))]
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

#[tauri::command(async)]
fn separation_offer(engine: State<Engine>) -> SeparationOffer {
    if Separators::offers(&engine.model_dirs(), SeparationQuality::BestVocals) {
        SeparationOffer::Choice
    } else {
        SeparationOffer::FastOnly {
            reason: "The vocal model isn't installed (scripts/fetch_models.py).".into(),
        }
    }
}

#[cfg(feature = "youtube")]
#[tauri::command(async)]
fn url_import(engine: State<Engine>) -> UrlImport {
    engine.extractor.status()
}

#[cfg(not(feature = "youtube"))]
#[tauri::command(async)]
fn url_import() -> UrlImport {
    UrlImport::Unavailable
}

/// The yt-dlp version (slow-ish: a standalone yt-dlp unpacks itself on every run).
#[cfg(feature = "youtube")]
#[tauri::command]
async fn extractor_version(app: AppHandle) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || app.state::<Engine>().extractor.version())
        .await
        .map_err(fail)?
}

#[cfg(feature = "youtube")]
#[tauri::command]
async fn update_extractor(app: AppHandle) -> Result<extractor::ExtractorUpdate, String> {
    tauri::async_runtime::spawn_blocking(move || app.state::<Engine>().extractor.update())
        .await
        .map_err(fail)?
}

#[cfg(not(feature = "youtube"))]
#[tauri::command]
async fn extractor_version() -> Result<String, String> {
    Err("this build has no YouTube import".into())
}

#[cfg(not(feature = "youtube"))]
#[tauri::command]
async fn update_extractor() -> Result<(), String> {
    Err("this build has no YouTube import".into())
}

#[cfg(feature = "youtube")]
#[tauri::command]
async fn import_url(
    app: AppHandle,
    url: String,
    quality: SeparationQuality,
) -> Result<Project, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let d = app.state::<Engine>().extractor.download(&url)?;
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

#[tauri::command(async)]
fn player(engine: State<Engine>, command: PlayerCommand) -> Result<(), String> {
    with_loaded(&engine, |l| {
        l.playback.send(command)?;
        if let PlayerCommand::SetMix { track, params } = command {
            // Mix settings are part of the project; `save_project` persists them.
            project::set_mix(&mut l.project, track, params);
        }
        Ok(())
    })
    .map_err(fail)
}

#[tauri::command(async)]
fn transport(engine: State<Engine>) -> Result<TransportSnapshot, String> {
    let session = engine.session.lock().map_err(fail)?;
    Ok(snapshot_of(&session))
}

fn snapshot_of(session: &Session) -> TransportSnapshot {
    match session {
        Session::Loaded(l) => TransportSnapshot {
            transport: l.playback.transport(),
            sample_rate: l.playback.sample_rate(),
        },
        Session::Empty => TransportSnapshot {
            transport: Transport::Stopped,
            sample_rate: WORKING_SAMPLE_RATE,
        },
    }
}

/// How often the transport is pushed to the UI while it moves (the UI extrapolates between).
const TRANSPORT_PUSH: std::time::Duration = std::time::Duration::from_millis(25);

/// Pushes the transport to the UI as `transport` events: every [`TRANSPORT_PUSH`] while the
/// playhead moves, and once on any other change. The UI never polls it per frame (AGENTS.md
/// §10.1). A session lock held by a long job skips a tick instead of waiting; the UI keeps
/// extrapolating the playhead meanwhile.
fn push_transport(app: AppHandle) {
    let mut last: Option<TransportSnapshot> = None;
    loop {
        std::thread::sleep(TRANSPORT_PUSH);
        let engine = app.state::<Engine>();
        let Ok(session) = engine.session.try_lock() else {
            continue;
        };
        let snap = snapshot_of(&session);
        drop(session);
        let moving = matches!(
            snap.transport,
            Transport::Playing { .. } | Transport::CountIn { .. } | Transport::Probing { .. }
        );
        if moving || last != Some(snap) {
            let _ = app.emit("transport", snap);
            last = Some(snap);
        }
    }
}

/// Imports a sung take from a file: lined up with the song by pitch, added, scored.
#[tauri::command]
async fn score_take(
    app: AppHandle,
    path: String,
    settings: ScoreSettings,
) -> Result<TakeAdded, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let engine = app.state::<Engine>();
        let project = with_loaded(&engine, |l| Ok(l.project.clone()))?;
        let path = PathBuf::from(path);
        let audio = decode::load(&path, project.sample_rate)?.to_mono();
        let raw = track_timeline(&engine, &project, &audio)?;
        let (timeline, placed) = take::timeline_from_file(&project, &audio, &raw.track)?;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let pitch = TrackedPitch {
            tracker: raw.tracker,
            track: placed,
        };
        add_take(
            &engine,
            TakeSource::Imported { name },
            &timeline,
            pitch,
            settings,
        )
    })
    .await
    .map_err(fail)?
    .map_err(fail)
}

/// Turns live monitoring off, or on dry or tuned. Opening the microphone at the output's rate
/// can fail (the UI shows why).
#[tauri::command(async)]
fn set_monitor(engine: State<Engine>, mode: MonitorMode) -> Result<MonitorStatus, String> {
    with_loaded(&engine, |l| {
        match (mode, &l.monitor) {
            (MonitorMode::Off, _) => {
                l.playback.send(PlayerCommand::Monitor { mode })?;
                l.playback.detach_monitor()?;
                l.monitor = Monitor::Off;
            }
            (MonitorMode::Dry | MonitorMode::Tuned, Monitor::Off) => {
                let (input, ring) = MonitorInput::start(l.playback.sample_rate())?;
                l.playback.attach_monitor(ring)?;
                l.monitor = Monitor::Listening { _input: input };
                l.playback.send(PlayerCommand::Monitor { mode })?;
            }
            (MonitorMode::Dry | MonitorMode::Tuned, Monitor::Listening { .. }) => {
                l.playback.send(PlayerCommand::Monitor { mode })?;
            }
        }
        let device = engine.latency.get(&device_pair(&l.playback)).ms;
        Ok(MonitorStatus::new(
            mode,
            l.playback.monitor_latency_ms(),
            device,
        ))
    })
    .map_err(fail)
}

/// The auto-tune settings: the live tuner's, and the project's (a save keeps them).
#[tauri::command(async)]
fn set_tune(engine: State<Engine>, settings: TuneSettings) -> Result<(), String> {
    with_loaded(&engine, |l| {
        l.project.tune = settings;
        l.playback.send(PlayerCommand::Tune { settings })
    })
    .map_err(fail)
}

/// What auto-tune would do to a take (from its stored pitch; no audio needed).
#[tauri::command(async)]
fn tune_preview(
    engine: State<Engine>,
    take: u32,
    settings: TuneSettings,
) -> Result<TunePreview, String> {
    with_loaded(&engine, |l| Ok(tune::preview(&l.project, take, settings)?)).map_err(fail)
}

/// Mono timeline audio of a pitch-editor source.
fn edit_audio(
    engine: &Engine,
    key: &str,
    project: &Project,
    source: EditSource,
) -> anyhow::Result<Vec<f32>> {
    match source {
        EditSource::Take { id } => engine.cache.take_audio(key, id),
        EditSource::VocalStem => engine.cache.vocal_audio(key, project),
    }
}

/// Opens a take or the vocal stem in the pitch editor (AGENTS.md §7.1).
#[tauri::command]
async fn pitch_open(app: AppHandle, source: EditSource) -> Result<PitchEditView, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let engine = app.state::<Engine>();
        let (key, project) =
            with_loaded(&engine, |l| Ok((l.cache_key.clone(), l.project.clone())))?;
        let audio = edit_audio(&engine, &key, &project, source)?;
        Ok(edit::open(&project, source, &audio)?)
    })
    .await
    .map_err(fail)?
    .map_err(|e: anyhow::Error| fail(e))
}

/// Applies one pitch-editor edit; the list is kept with the project.
#[tauri::command(async)]
fn pitch_apply(engine: State<Engine>, edit: PitchEdit, op: EditOp) -> Result<EditUpdate, String> {
    with_loaded(&engine, |l| Ok(edit::apply_op(&mut l.project, edit, &op)?)).map_err(fail)
}

/// Replaces a source's edit list (undo and redo).
#[tauri::command(async)]
fn pitch_set(engine: State<Engine>, edit: PitchEdit) -> Result<EditUpdate, String> {
    with_loaded(&engine, |l| Ok(edit::store(&mut l.project, edit)?)).map_err(fail)
}

/// Seeds a take's edit list from auto-tune's settings (the editable correction curve).
#[tauri::command(async)]
fn pitch_from_tune(
    engine: State<Engine>,
    take: u32,
    settings: TuneSettings,
) -> Result<EditUpdate, String> {
    with_loaded(&engine, |l| {
        Ok(edit::seed_from_tune(&mut l.project, take, settings)?)
    })
    .map_err(fail)
}

/// The model manager's list: every manifest model and where it is on this device.
#[tauri::command(async)]
fn models_list(engine: State<Engine>) -> Vec<ModelEntry> {
    engine.models.lock().expect("models lock").entries()
}

/// A model state change, for the model manager's progress.
#[derive(Clone, Serialize)]
struct ModelProgress {
    id: String,
    state: ModelState,
}

/// Downloads (and verifies) model `id` into app data. Opt-in (restricted) models need the user
/// to have accepted the licence. Returns the final state; progress arrives as "model-state".
#[tauri::command]
async fn model_download(
    app: AppHandle,
    id: String,
    accept_license: bool,
) -> Result<ModelState, String> {
    let info = models::model(&id).ok_or_else(|| format!("unknown model {id}"))?;
    if info.distribution == Distribution::OptIn && !accept_license {
        return Err(format!(
            "{} is under {}: accept its licence first",
            info.id, info.license
        ));
    }
    tauri::async_runtime::spawn_blocking(move || {
        let engine = app.state::<Engine>();
        let dir = engine
            .models
            .lock()
            .expect("models lock")
            .download_dir()
            .to_path_buf();
        let _ = engine_native::models::download(&info, &dir, |event| {
            let state = engine
                .models
                .lock()
                .expect("models lock")
                .advance(&info, event);
            let _ = app.emit(
                "model-state",
                ModelProgress {
                    id: info.id.clone(),
                    state,
                },
            );
        });
        models_changed(&engine);
        let state = engine.models.lock().expect("models lock").state(&info);
        state
    })
    .await
    .map_err(fail)
}

/// Deletes a downloaded model's files from app data.
#[tauri::command(async)]
fn model_delete(engine: State<Engine>, id: String) -> Result<ModelState, String> {
    let info = models::model(&id).ok_or_else(|| format!("unknown model {id}"))?;
    let state = engine
        .models
        .lock()
        .expect("models lock")
        .delete(&info)
        .map_err(fail)?;
    models_changed(&engine);
    Ok(state)
}

/// Installed models changed: separation and pitch look for their weights again.
fn models_changed(engine: &Engine) {
    let dirs = engine.model_dirs();
    *engine.model.lock().expect("model lock") = Model::NotLoaded;
    *engine.pitch.lock().expect("pitch lock") = Pitch::find(&dirs);
}

/// Transcribes a track of the loaded song to MIDI (basic-pitch, or the drum classifier) and
/// keeps it with the project.
#[tauri::command]
async fn transcribe(app: AppHandle, source: TrackSource) -> Result<TranscribeOutcome, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let engine = app.state::<Engine>();
        let (key, rate) = with_loaded(&engine, |l| {
            Ok((l.cache_key.clone(), l.project.sample_rate))
        })?;
        let model = engine
            .models
            .lock()
            .expect("models lock")
            .find(basic_pitch::MODEL_FILE);
        let audio = engine.cache.track_audio(&key, source)?;
        let t = match engine_native::transcribe::transcribe(source, &audio, rate, model.as_deref())
        {
            Ok(t) => t,
            Err(Missing::Model) => {
                return Ok(TranscribeOutcome::NeedsModel {
                    model: midi::BASIC_PITCH_MODEL.to_string(),
                })
            }
            Err(Missing::Failed(e)) => return Err(e),
        };
        with_loaded(&engine, |l| {
            midi::store(&mut l.project, t);
            Ok(TranscribeOutcome::Done {
                project: Box::new(l.project.clone()),
            })
        })
    })
    .await
    .map_err(fail)?
    .map_err(fail)
}

/// Writes a track's MIDI layer to `path` as a Standard MIDI File.
#[tauri::command(async)]
fn export_midi(engine: State<Engine>, source: TrackSource, path: PathBuf) -> Result<(), String> {
    let bytes = with_loaded(&engine, |l| {
        let t = midi::transcription(&l.project, source).context("this track has no MIDI yet")?;
        Ok(midi::smf(t, l.project.analysis.tempo.bpm))
    })
    .map_err(fail)?;
    std::fs::write(path, bytes).map_err(fail)
}

/// Re-scores a saved take (from its stored pitch; no audio needed).
#[tauri::command(async)]
fn score_recorded(
    engine: State<Engine>,
    take: u32,
    settings: ScoreSettings,
) -> Result<ScoreReport, String> {
    with_loaded(&engine, |l| {
        Ok(take::score_take(&l.project, take, settings)?)
    })
    .map_err(fail)
}

/// Opens the microphone and starts the count-in from the current position.
#[tauri::command(async)]
fn start_recording(engine: State<Engine>) -> Result<(), String> {
    with_loaded(&engine, |l| {
        anyhow::ensure!(matches!(l.mic, Mic::Closed), "the microphone is busy");
        let capture = Capture::start()?;
        let rate = l.playback.sample_rate();
        let count_in = cue::count_in(l.project.analysis.tempo.bpm, rate);
        let PlayerCommand::CountIn { beats, beat_frames } = count_in else {
            unreachable!("cue::count_in builds a count-in")
        };
        l.playback.clear_cues();
        l.playback.send(count_in)?;
        // The count-in holds the song position it started from (a playing song moves on
        // between a transport read and the callback applying the command).
        let started = Instant::now();
        let from = loop {
            match l.playback.transport() {
                Transport::CountIn { frame, .. } => break frame,
                _ if started.elapsed() > CUE_TIMEOUT => anyhow::bail!("the count-in didn't start"),
                _ => std::thread::sleep(Duration::from_millis(2)),
            }
        };
        l.mic = Mic::Recording {
            capture,
            from_seconds: f64::from(from) / f64::from(rate),
            count_in_seconds: f64::from(beats * beat_frames) / f64::from(rate),
            live: Live::Waiting,
        };
        Ok(())
    })
    .map_err(fail)
}

/// Peak input level of the take being recorded, 0–1.
#[tauri::command(async)]
fn capture_level(engine: State<Engine>) -> f32 {
    with_loaded(&engine, |l| {
        Ok(match &l.mic {
            Mic::Recording { capture, .. } => capture.level(),
            Mic::Closed | Mic::Calibrating => 0.0,
        })
    })
    .unwrap_or(0.0)
}

/// Stops the take being recorded, lines it up with the song, adds and scores it.
#[tauri::command]
async fn finish_recording(app: AppHandle, settings: ScoreSettings) -> Result<TakeAdded, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let engine = app.state::<Engine>();
        let (capture, from_seconds, count_in_seconds, cue, latency, project) =
            with_loaded(&engine, |l| {
                let Mic::Recording {
                    capture,
                    from_seconds,
                    count_in_seconds,
                    ..
                } = std::mem::replace(&mut l.mic, Mic::Closed)
                else {
                    anyhow::bail!("no take is being recorded");
                };
                if !matches!(l.playback.transport(), Transport::Stopped) {
                    l.playback.send(PlayerCommand::Stop)?;
                }
                let cue = l
                    .playback
                    .take_cue()
                    .ok_or_else(|| anyhow::anyhow!("the count-in never started"))?;
                let latency = engine
                    .latency
                    .get(&latency::devices(capture.device(), l.playback.device()));
                Ok((
                    capture,
                    from_seconds,
                    count_in_seconds,
                    cue,
                    latency,
                    l.project.clone(),
                ))
            })?;
        let captured = capture.finish()?;
        let song_zero = song_zero(
            cue,
            captured.first,
            count_in_seconds,
            &latency,
            from_seconds,
        );
        let timeline = take::timeline_from_capture(
            &project,
            &captured.samples,
            captured.sample_rate,
            song_zero,
        )?;
        let pitch = track_timeline(&engine, &project, &timeline)?;
        add_take(
            &engine,
            TakeSource::Recorded {
                latency_ms: latency.ms,
            },
            &timeline,
            pitch,
            settings,
        )
    })
    .await
    .map_err(fail)?
    .map_err(fail)
}

/// Live pitch of the take being recorded since the previous call, compared with the reference.
#[tauri::command(async)]
fn live_pitch(engine: State<Engine>, settings: ScoreSettings) -> Result<LiveUpdate, String> {
    with_loaded(&engine, |l| {
        let Mic::Recording {
            capture,
            from_seconds,
            count_in_seconds,
            live,
        } = &mut l.mic
        else {
            return Ok(LiveUpdate::default());
        };
        if let Live::Waiting = live {
            let (Some(cue), Some(first)) = (l.playback.peek_cue(), capture.first_instant()) else {
                return Ok(LiveUpdate {
                    input: capture.state(),
                    ..LiveUpdate::default()
                });
            };
            let latency = engine
                .latency
                .get(&latency::devices(capture.device(), l.playback.device()));
            *live = Live::Running {
                session: Box::new(LiveSession::new(
                    &l.project.analysis.reference_pitch,
                    f64::from(l.project.frames) / f64::from(l.project.sample_rate),
                    settings,
                    capture.sample_rate(),
                    song_zero(cue, first, *count_in_seconds, &latency, *from_seconds),
                )),
                read: 0,
            };
        }
        let Live::Running { session, read } = live else {
            unreachable!("set above")
        };
        let samples = capture.samples_since(*read);
        *read += samples.len();
        Ok(LiveUpdate {
            input: capture.state(),
            ..session.push(&samples)
        })
    })
    .map_err(fail)
}

/// Sets the loaded song's lyrics (fetched, not found, or edited) and returns them timed.
#[tauri::command(async)]
fn set_lyrics(engine: State<Engine>, update: LyricsUpdate) -> Result<ProjectLyrics, String> {
    with_loaded(&engine, |l| Ok(lyrics::apply(&mut l.project, update)?)).map_err(fail)
}

/// Abandons the take being recorded.
#[tauri::command(async)]
fn cancel_recording(engine: State<Engine>) -> Result<(), String> {
    with_loaded(&engine, |l| {
        if let Mic::Recording { .. } = l.mic {
            l.mic = Mic::Closed;
            if !matches!(l.playback.transport(), Transport::Stopped) {
                l.playback.send(PlayerCommand::Stop)?;
            }
        }
        Ok(())
    })
    .map_err(fail)
}

/// The round-trip latency removed from takes on the current devices.
#[tauri::command(async)]
fn latency(engine: State<Engine>) -> Result<Latency, String> {
    with_loaded(&engine, |l| {
        Ok(engine.latency.get(&device_pair(&l.playback)))
    })
    .map_err(fail)
}

/// Sets the latency by hand (the ± nudge).
#[tauri::command(async)]
fn set_latency(engine: State<Engine>, ms: f32) -> Result<Latency, String> {
    with_loaded(&engine, |l| {
        let latency = Latency {
            ms: ms.clamp(0.0, cue::MAX_LATENCY_MS),
            source: LatencySource::Manual,
            devices: device_pair(&l.playback),
        };
        engine.latency.set(&latency)?;
        Ok(latency)
    })
    .map_err(fail)
}

/// Loopback calibration: plays the chirp, records it, and stores the measured delay.
#[tauri::command]
async fn measure_latency(app: AppHandle) -> Result<Latency, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let engine = app.state::<Engine>();
        let (capture, devices) = with_loaded(&engine, |l| {
            anyhow::ensure!(matches!(l.mic, Mic::Closed), "the microphone is busy");
            let capture = Capture::start()?;
            match l.playback.transport() {
                Transport::Playing { .. } => l.playback.send(PlayerCommand::Pause)?,
                Transport::CountIn { .. } | Transport::Probing { .. } => {
                    l.playback.send(PlayerCommand::Stop)?
                }
                Transport::Stopped | Transport::Paused { .. } => {}
            }
            l.playback.clear_cues();
            l.playback.send(PlayerCommand::Probe)?;
            l.mic = Mic::Calibrating;
            let devices = latency::devices(capture.device(), l.playback.device());
            Ok((capture, devices))
        })?;
        let measured = (|| {
            let started = Instant::now();
            // Wait for the chirp to start and finish, then keep listening for its echo.
            loop {
                std::thread::sleep(POLL);
                let done = with_loaded(&engine, |l| {
                    Ok(!matches!(l.playback.transport(), Transport::Probing { .. }))
                })?;
                if done && started.elapsed() > POLL * 3 {
                    break;
                }
                anyhow::ensure!(started.elapsed() < PROBE_TIMEOUT, "the chirp didn't play");
            }
            std::thread::sleep(Duration::from_secs_f64(PROBE_TAIL_SECONDS));
            let cue = with_loaded(&engine, |l| {
                l.playback
                    .take_cue()
                    .ok_or_else(|| anyhow::anyhow!("the chirp didn't play"))
            })?;
            let captured = capture.finish()?;
            let at = capture::seconds_between(cue, captured.first);
            let (ms, confidence) =
                cue::measure_latency(&captured.samples, captured.sample_rate, at)?;
            let latency = Latency {
                ms,
                source: LatencySource::Measured { confidence },
                devices,
            };
            engine.latency.set(&latency)?;
            Ok(latency)
        })();
        // The microphone is free again whatever happened.
        let _ = with_loaded(&engine, |l| {
            l.mic = Mic::Closed;
            Ok(())
        });
        measured
    })
    .await
    .map_err(fail)?
    .map_err(fail)
}

/// The system's default microphone, which every recording, calibration and the monitor opens.
#[tauri::command(async)]
fn input_device() -> InputState {
    capture::input_state()
}

/// Opens System Settings › Sound › Input, where macOS users pick their microphone (an iPhone via
/// Continuity included). Other platforms have no picker yet (AGENTS.md §11).
#[tauri::command(async)]
fn open_input_settings() -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("Pick the microphone in your system's sound settings.".into());
    }
    let open = |target: &str| {
        std::process::Command::new("open")
            .arg(target)
            .status()
            .is_ok_and(|s| s.success())
    };
    if open("x-apple.systempreferences:com.apple.Sound-Settings.extension?input")
        || open("x-apple.systempreferences:com.apple.preference.sound")
    {
        Ok(())
    } else {
        Err("Couldn't open System Settings › Sound.".into())
    }
}

/// Appends every panic (message, location, thread) to `path`, so a crash in the field leaves a
/// trace. Logs the first 50 per run: a panic repeating on an audio thread mustn't fill the disk.
fn log_panics(path: PathBuf) {
    use std::io::Write;
    use std::sync::atomic::{AtomicU32, Ordering};
    static LOGGED: AtomicU32 = AtomicU32::new(0);
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if LOGGED.fetch_add(1, Ordering::Relaxed) < 50 {
            let thread = std::thread::current();
            let when = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let _ = std::fs::create_dir_all(path.parent().unwrap_or(Path::new(".")));
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            {
                let _ = writeln!(
                    f,
                    "[{when}] thread {:?}: {info}",
                    thread.name().unwrap_or("unnamed")
                );
            }
        }
        previous(info);
    }));
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let data = app.path().app_data_dir()?;
            log_panics(data.join("logs").join("panic.log"));
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
                latency: LatencyStore::new(data.join("latency.json")),
                models: Mutex::new(ModelStore::new(model_dirs)),
                generator: Mutex::new(produce::Generator::new(
                    data.join("ml-sidecar"),
                    app.path().resource_dir().ok(),
                )),
                #[cfg(feature = "youtube")]
                extractor: extractor::Extractor::new(&data),
            });
            let handle = app.handle().clone();
            std::thread::spawn(move || push_transport(handle));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            rename_take,
            delete_take,
            restore_take,
            replace_take,
            import_file,
            separation_offer,
            url_import,
            extractor_version,
            update_extractor,
            import_url,
            player,
            transport,
            score_take,
            score_recorded,
            start_recording,
            capture_level,
            live_pitch,
            set_lyrics,
            finish_recording,
            cancel_recording,
            latency,
            set_latency,
            measure_latency,
            input_device,
            open_input_settings,
            set_monitor,
            set_tune,
            tune_preview,
            pitch_open,
            pitch_apply,
            pitch_set,
            pitch_from_tune,
            models_list,
            model_download,
            model_delete,
            transcribe,
            export_midi,
            produce::chain_pick,
            produce::harmony,
            produce::export_melody,
            chain::set_take_chain,
            chain::set_master,
            export::export_formats,
            export::export_mix,
            export::export_archive,
            export::import_archive,
            produce::generator_state,
            produce::generator_install,
            produce::generator_uninstall,
            produce::generate,
            list_projects,
            open_project,
            save_project,
            close_session
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
