//! Tauri desktop host: exposes engine-native to the shared Svelte UI.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use engine_core::cue::{self, Latency, LatencySource, PROBE_TAIL_SECONDS};
use engine_core::live::{LiveSession, LiveUpdate};
use engine_core::lyrics::{self, LyricsUpdate, ProjectLyrics};
use engine_core::player::{PlayerCommand, Transport, TransportSnapshot};
use engine_core::project::{
    self, ImportStage, Project, ProjectSummary, SeparationQuality, SongAudio, SongSource,
    TrackedPitch,
};
use engine_core::score::{ScoreReport, ScoreSettings};
use engine_core::take::{self, TakeAdded, TakeSource};
use engine_core::WORKING_SAMPLE_RATE;
use engine_native::cache::{Lookup, StemCache};
use engine_native::capture::{self, Capture, StreamInstant};
use engine_native::decode;
use engine_native::import::{import_song, Imported, Separation};
use engine_native::latency::{self, LatencyStore};
use engine_native::pitch::Pitch;
use engine_native::playback::Playback;
use engine_native::separate::Separators;
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
    start_session(&engine, cache_key, project, &audio, &takes)
}

/// Makes `project` the loaded song and starts its playback stream.
fn start_session(
    engine: &Engine,
    cache_key: String,
    project: Project,
    audio: &SongAudio,
    takes: &[Vec<f32>],
) -> anyhow::Result<Project> {
    let mut session = engine.session.lock().expect("session lock");
    // Close the old stream before opening a new one.
    *session = Session::Empty;
    let playback = Playback::start(&project, audio, takes)?;
    *session = Session::Loaded(Box::new(Loaded {
        cache_key,
        project: project.clone(),
        playback,
        mic: Mic::Closed,
    }));
    Ok(project)
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
            } => start_session(&engine, id, *project, &audio, &takes),
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
    with_loaded(&engine, |l| {
        engine.cache.save_project(&l.cache_key, &l.project)
    })
    .map_err(fail)
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

#[tauri::command]
fn transport(engine: State<Engine>) -> Result<TransportSnapshot, String> {
    let session = engine.session.lock().map_err(fail)?;
    match &*session {
        Session::Loaded(l) => Ok(TransportSnapshot {
            transport: l.playback.transport(),
            sample_rate: l.playback.sample_rate(),
        }),
        Session::Empty => Ok(TransportSnapshot {
            transport: Transport::Stopped,
            sample_rate: WORKING_SAMPLE_RATE,
        }),
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

/// Re-scores a saved take (from its stored pitch; no audio needed).
#[tauri::command]
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
#[tauri::command]
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
#[tauri::command]
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
#[tauri::command]
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
                return Ok(LiveUpdate::default());
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
        Ok(session.push(&samples))
    })
    .map_err(fail)
}

/// Sets the loaded song's lyrics (fetched, not found, or edited) and returns them timed.
#[tauri::command]
fn set_lyrics(engine: State<Engine>, update: LyricsUpdate) -> Result<ProjectLyrics, String> {
    with_loaded(&engine, |l| Ok(lyrics::apply(&mut l.project, update)?)).map_err(fail)
}

/// Abandons the take being recorded.
#[tauri::command]
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
#[tauri::command]
fn latency(engine: State<Engine>) -> Result<Latency, String> {
    with_loaded(&engine, |l| {
        Ok(engine.latency.get(&device_pair(&l.playback)))
    })
    .map_err(fail)
}

/// Sets the latency by hand (the ± nudge).
#[tauri::command]
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
                latency: LatencyStore::new(data.join("latency.json")),
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
            list_projects,
            open_project,
            save_project,
            close_session
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
