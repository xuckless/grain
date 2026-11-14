//! Web bindings. Two consumers:
//! - the main thread: [`WasmSeparator`] → [`WasmSong`] (import, analysis, scoring);
//! - the AudioWorklet: [`WasmPlayer`] (real-time mixing; no allocation per block).

use engine_core::audio::StereoAudio;
use engine_core::cue;
use engine_core::edit::{self, EditOp, EditSource};
use engine_core::live::LiveSession;
use engine_core::lyrics::{self, LyricsUpdate};
use engine_core::melody::Note;
use engine_core::player::{MonitorStatus, Player, PlayerCommand, PlayerTrack};
use engine_core::project::{self, Project, SongAudio, SongSource, TrackedPitch};
use engine_core::score::ScoreSettings;
use engine_core::take::{self, TakeAdded, TakeSource};
use engine_core::tune;
use engine_ml::demucs::{ModelSpec, Separator, SeparatorState};
use wasm_bindgen::prelude::*;

/// Largest block the worklet renders at once (Web Audio uses 128).
const MAX_BLOCK: usize = 1024;

fn to_js<T: serde::Serialize>(value: &T) -> Result<JsValue, JsError> {
    value
        .serialize(&serde_wasm_bindgen::Serializer::json_compatible())
        .map_err(|e| JsError::new(&e.to_string()))
}

fn from_js<T: serde::de::DeserializeOwned>(value: JsValue) -> Result<T, JsError> {
    serde_wasm_bindgen::from_value(value).map_err(|e| JsError::new(&e.to_string()))
}

fn stereo(left: Vec<f32>, right: Vec<f32>, sample_rate: u32) -> Result<StereoAudio, JsError> {
    if left.len() != right.len() {
        return Err(JsError::new("channel lengths differ"));
    }
    Ok(StereoAudio {
        sample_rate,
        left,
        right,
    })
}

fn js_err(e: impl std::fmt::Display) -> JsError {
    JsError::new(&e.to_string())
}

/// An imported or reopened song: its audio, the analyzed project, and the timeline audio of each
/// take. The web host saves all three to OPFS (AGENTS.md §12).
#[wasm_bindgen]
pub struct WasmSong {
    project: Project,
    audio: SongAudio,
    takes: Vec<Vec<f32>>,
}

impl WasmSong {
    fn new(name: String, audio: SongAudio) -> WasmSong {
        WasmSong {
            // RMVPE (361 MB) is desktop-only; the web tracks pitch with YIN (AGENTS.md §5).
            project: project::build(
                SongSource::File { name },
                &audio,
                TrackedPitch::yin(
                    &project::reference_signal(&audio),
                    project::track_audio(&audio)[0].sample_rate,
                ),
            ),
            audio,
            takes: Vec::new(),
        }
    }

    /// Adds a take (timeline audio) and returns the updated project with the take's score.
    fn add(
        &mut self,
        source: TakeSource,
        timeline: Vec<f32>,
        pitch: TrackedPitch,
        settings: ScoreSettings,
    ) -> Result<JsValue, JsError> {
        let id = take::add_take(&mut self.project, source, &timeline, pitch);
        self.takes.push(timeline);
        to_js(&TakeAdded {
            report: take::score_take(&self.project, id, settings).map_err(js_err)?,
            project: self.project.clone(),
            take: id,
        })
    }

    /// Mono timeline audio of a pitch-editor source.
    fn edit_audio(&self, source: EditSource) -> Result<Vec<f32>, JsError> {
        match source {
            EditSource::Take { id } => Ok(self.timeline(id)?.to_vec()),
            EditSource::VocalStem => Ok(project::reference_signal(&self.audio)),
        }
    }

    /// Timeline audio of take `id`.
    fn timeline(&self, id: u32) -> Result<&[f32], JsError> {
        let i = self
            .project
            .takes
            .iter()
            .position(|t| t.id == id)
            .ok_or_else(|| JsError::new("no such take"))?;
        Ok(&self.takes[i])
    }
}

#[wasm_bindgen]
impl WasmSong {
    /// Builds a song without separation (no model available).
    pub fn unseparated(
        name: String,
        left: Vec<f32>,
        right: Vec<f32>,
        sample_rate: u32,
    ) -> Result<WasmSong, JsError> {
        let mix = stereo(left, right, sample_rate)?;
        Ok(WasmSong::new(name, SongAudio::Unseparated { mix }))
    }

    /// The `Project` (see packages/engine-api).
    pub fn project(&self) -> Result<JsValue, JsError> {
        to_js(&self.project)
    }

    /// `[left, right]` of project track `index`.
    #[wasm_bindgen(js_name = trackAudio)]
    pub fn track_audio(&self, index: usize) -> Result<js_sys::Array, JsError> {
        let tracks = project::track_audio(&self.audio);
        let audio = tracks
            .get(index)
            .ok_or_else(|| JsError::new("no such track"))?;
        let out = js_sys::Array::new();
        out.push(&js_sys::Float32Array::from(audio.left.as_slice()));
        out.push(&js_sys::Float32Array::from(audio.right.as_slice()));
        Ok(out)
    }

    /// Stores a player track's mix (song tracks, then takes) so a save keeps it.
    #[wasm_bindgen(js_name = setMix)]
    pub fn set_mix(&mut self, player_track: u32, params: JsValue) -> Result<(), JsError> {
        project::set_mix(&mut self.project, player_track, from_js(params)?);
        Ok(())
    }

    /// Adds a take file (same sample rate as the song), lined up by pitch, and scores it.
    /// Returns a `TakeAdded`.
    #[wasm_bindgen(js_name = addFileTake)]
    pub fn add_file_take(
        &mut self,
        name: String,
        left: Vec<f32>,
        right: Vec<f32>,
        settings: JsValue,
    ) -> Result<JsValue, JsError> {
        let settings: ScoreSettings = from_js(settings)?;
        let rate = self.project.sample_rate;
        let audio = stereo(left, right, rate)?.to_mono();
        let raw = TrackedPitch::yin(&audio, rate);
        let (timeline, placed) =
            take::timeline_from_file(&self.project, &audio, &raw.track).map_err(js_err)?;
        let pitch = TrackedPitch {
            tracker: raw.tracker,
            track: placed,
        };
        self.add(TakeSource::Imported { name }, timeline, pitch, settings)
    }

    /// Adds a microphone capture as a take. `song_zero_seconds`: capture time at which song
    /// frame 0 would have been heard (see `take::timeline_from_capture`). Returns a `TakeAdded`.
    #[wasm_bindgen(js_name = addRecordedTake)]
    pub fn add_recorded_take(
        &mut self,
        capture: Vec<f32>,
        capture_rate: u32,
        song_zero_seconds: f64,
        latency_ms: f32,
        settings: JsValue,
    ) -> Result<JsValue, JsError> {
        let settings: ScoreSettings = from_js(settings)?;
        let timeline =
            take::timeline_from_capture(&self.project, &capture, capture_rate, song_zero_seconds)
                .map_err(js_err)?;
        let pitch = TrackedPitch::yin(&timeline, self.project.sample_rate);
        self.add(
            TakeSource::Recorded { latency_ms },
            timeline,
            pitch,
            settings,
        )
    }

    /// Sets the song's lyrics (a `LyricsUpdate`); returns the timed `ProjectLyrics`.
    #[wasm_bindgen(js_name = setLyrics)]
    pub fn set_lyrics(&mut self, update: JsValue) -> Result<JsValue, JsError> {
        let update: LyricsUpdate = from_js(update)?;
        to_js(&lyrics::apply(&mut self.project, update).map_err(js_err)?)
    }

    /// Starts live karaoke pitch for a capture whose song time 0 is at `song_zero_seconds`
    /// (same placement as [`WasmSong::add_recorded_take`]).
    #[wasm_bindgen(js_name = liveSession)]
    pub fn live_session(
        &self,
        settings: JsValue,
        capture_rate: u32,
        song_zero_seconds: f64,
    ) -> Result<WasmLive, JsError> {
        Ok(WasmLive {
            session: LiveSession::new(
                &self.project.analysis.reference_pitch,
                f64::from(self.project.frames) / f64::from(self.project.sample_rate),
                from_js(settings)?,
                capture_rate,
                song_zero_seconds,
            ),
        })
    }

    /// Re-scores take `id`; returns a `ScoreReport`.
    #[wasm_bindgen(js_name = scoreTake)]
    pub fn score_take(&self, id: u32, settings: JsValue) -> Result<JsValue, JsError> {
        let settings: ScoreSettings = from_js(settings)?;
        to_js(&take::score_take(&self.project, id, settings).map_err(js_err)?)
    }

    /// Timeline audio of take `id` (for the player).
    #[wasm_bindgen(js_name = takeAudio)]
    pub fn take_audio(&self, id: u32) -> Result<js_sys::Float32Array, JsError> {
        Ok(js_sys::Float32Array::from(self.timeline(id)?))
    }

    /// Stores the auto-tune settings (a `TuneSettings`) so a save keeps them.
    #[wasm_bindgen(js_name = setTune)]
    pub fn set_tune(&mut self, settings: JsValue) -> Result<(), JsError> {
        self.project.tune = from_js(settings)?;
        Ok(())
    }

    /// What auto-tune would do to take `id` with `settings` (a `TuneSettings`); returns a
    /// `TunePreview`.
    #[wasm_bindgen(js_name = tunePreview)]
    pub fn tune_preview(&self, id: u32, settings: JsValue) -> Result<JsValue, JsError> {
        to_js(&tune::preview(&self.project, id, from_js(settings)?).map_err(js_err)?)
    }

    /// Adds take `id` auto-tuned as a new take, "<take> (tuned)", and scores it. Returns a
    /// `TakeAdded`.
    #[wasm_bindgen(js_name = applyTune)]
    pub fn apply_tune(
        &mut self,
        id: u32,
        tune: JsValue,
        settings: JsValue,
    ) -> Result<JsValue, JsError> {
        let settings: ScoreSettings = from_js(settings)?;
        let tuned =
            tune::render(&self.project, id, self.timeline(id)?, from_js(tune)?).map_err(js_err)?;
        let pitch = TrackedPitch::yin(&tuned, self.project.sample_rate);
        self.add(TakeSource::Tuned { from: id }, tuned, pitch, settings)
    }

    /// Opens a pitch-editor source (an `EditSource`); returns a `PitchEditView`.
    #[wasm_bindgen(js_name = pitchOpen)]
    pub fn pitch_open(&self, source: JsValue) -> Result<JsValue, JsError> {
        let source: EditSource = from_js(source)?;
        let audio = self.edit_audio(source)?;
        to_js(&edit::open(&self.project, source, &audio).map_err(js_err)?)
    }

    /// Applies an `EditOp` to a `PitchEdit`, keeping it with the project; returns an `EditUpdate`.
    #[wasm_bindgen(js_name = pitchApply)]
    pub fn pitch_apply(&mut self, edit: JsValue, op: JsValue) -> Result<JsValue, JsError> {
        let op: EditOp = from_js(op)?;
        to_js(&edit::apply_op(&mut self.project, from_js(edit)?, &op).map_err(js_err)?)
    }

    /// Replaces a source's edit list (undo and redo); returns an `EditUpdate`.
    #[wasm_bindgen(js_name = pitchSet)]
    pub fn pitch_set(&mut self, edit: JsValue) -> Result<JsValue, JsError> {
        to_js(&edit::store(&mut self.project, from_js(edit)?).map_err(js_err)?)
    }

    /// Seeds take `id`'s edit list from auto-tune `settings`; returns an `EditUpdate`.
    #[wasm_bindgen(js_name = pitchFromTune)]
    pub fn pitch_from_tune(&mut self, id: u32, settings: JsValue) -> Result<JsValue, JsError> {
        to_js(&edit::seed_from_tune(&mut self.project, id, from_js(settings)?).map_err(js_err)?)
    }

    /// Renders a source's edits as a new take, "<source> (edited)", and scores it. Returns a
    /// `TakeAdded`.
    #[wasm_bindgen(js_name = pitchRender)]
    pub fn pitch_render(&mut self, source: JsValue, settings: JsValue) -> Result<JsValue, JsError> {
        let source: EditSource = from_js(source)?;
        let settings: ScoreSettings = from_js(settings)?;
        let audio = self.edit_audio(source)?;
        let edited = edit::render_source(&self.project, source, &audio).map_err(js_err)?;
        let pitch = TrackedPitch::yin(&edited, self.project.sample_rate);
        self.add(TakeSource::Edited { from: source }, edited, pitch, settings)
    }
}

/// Reopens a saved project one channel at a time (so JS never holds the whole song twice):
/// push each project track's left and right channel, then each take's timeline audio, in
/// project order, then `finish`.
#[wasm_bindgen]
pub struct SongRestorer {
    project: Project,
    tracks: Vec<StereoAudio>,
    takes: Vec<Vec<f32>>,
}

#[wasm_bindgen]
impl SongRestorer {
    #[wasm_bindgen(constructor)]
    pub fn new(project: JsValue) -> Result<SongRestorer, JsError> {
        Ok(SongRestorer {
            project: from_js(project)?,
            tracks: Vec::new(),
            takes: Vec::new(),
        })
    }

    #[wasm_bindgen(js_name = pushTrack)]
    pub fn push_track(&mut self, left: Vec<f32>, right: Vec<f32>) -> Result<(), JsError> {
        self.tracks
            .push(stereo(left, right, self.project.sample_rate)?);
        Ok(())
    }

    #[wasm_bindgen(js_name = pushTake)]
    pub fn push_take(&mut self, audio: Vec<f32>) {
        self.takes.push(audio);
    }

    pub fn finish(mut self) -> Result<WasmSong, JsError> {
        if self.takes.len() != self.project.takes.len() {
            return Err(JsError::new("the saved takes don't match the project"));
        }
        let audio = project::restore_audio(&mut self.project, self.tracks).map_err(js_err)?;
        Ok(WasmSong {
            project: self.project,
            audio,
            takes: self.takes,
        })
    }
}

/// A saved project's entry in the recent list.
#[wasm_bindgen(js_name = projectSummary)]
pub fn project_summary(project: JsValue, id: String, saved_ms: f64) -> Result<JsValue, JsError> {
    let project: Project = from_js(project)?;
    to_js(&project::ProjectSummary::new(id, &project, saved_ms))
}

/// The count-in `PlayerCommand` for a song at `bpm` played at `sample_rate`.
#[wasm_bindgen(js_name = countIn)]
pub fn count_in(bpm: f64, sample_rate: u32) -> Result<JsValue, JsError> {
    to_js(&cue::count_in(bpm, sample_rate))
}

/// The `MonitorStatus` for `mode` (a `MonitorMode`): `shifter_ms` is the player's
/// `monitorLatency` in ms, `device_ms` the calibrated round trip.
#[wasm_bindgen(js_name = monitorStatus)]
pub fn monitor_status(mode: JsValue, shifter_ms: f32, device_ms: f32) -> Result<JsValue, JsError> {
    to_js(&MonitorStatus::new(from_js(mode)?, shifter_ms, device_ms))
}

/// Round-trip latency from a calibration capture: `[ms, confidence]`, or an error saying why
/// the chirp wasn't usable.
#[wasm_bindgen(js_name = measureLatency)]
pub fn measure_latency(
    capture: Vec<f32>,
    capture_rate: u32,
    cue_at_seconds: f64,
) -> Result<Vec<f32>, JsError> {
    let (ms, confidence) =
        cue::measure_latency(&capture, capture_rate, cue_at_seconds).map_err(js_err)?;
    Ok(vec![ms, confidence])
}

/// HTDemucs host processing driven from JS, which runs the model with onnxruntime-web.
#[wasm_bindgen]
pub struct WasmSeparator {
    name: String,
    separator: Separator,
}

#[wasm_bindgen]
impl WasmSeparator {
    #[wasm_bindgen(constructor)]
    pub fn new(
        spec_json: &str,
        name: String,
        left: Vec<f32>,
        right: Vec<f32>,
    ) -> Result<WasmSeparator, JsError> {
        let spec = ModelSpec::parse(spec_json).map_err(|e| JsError::new(&e.to_string()))?;
        let mix = stereo(left, right, spec.samplerate)?;
        Ok(WasmSeparator {
            name,
            separator: Separator::new(spec, &mix),
        })
    }

    /// `{ kind: "AwaitingSegment", index, count }` or `{ kind: "Complete" }`.
    pub fn state(&self) -> Result<JsValue, JsError> {
        #[derive(serde::Serialize)]
        #[serde(tag = "kind")]
        enum State {
            AwaitingSegment { index: usize, count: usize },
            Complete,
        }
        to_js(&match self.separator.state() {
            SeparatorState::AwaitingSegment { index, count } => {
                State::AwaitingSegment { index, count }
            }
            SeparatorState::Complete => State::Complete,
        })
    }

    /// `[mix, mag]` for the awaited segment.
    pub fn inputs(&self) -> Result<js_sys::Array, JsError> {
        let inputs = self
            .separator
            .inputs()
            .map_err(|e| JsError::new(&e.to_string()))?;
        let out = js_sys::Array::new();
        out.push(&js_sys::Float32Array::from(inputs.mix.as_slice()));
        out.push(&js_sys::Float32Array::from(inputs.mag.as_slice()));
        Ok(out)
    }

    pub fn submit(&mut self, spec: Vec<f32>, wav: Vec<f32>) -> Result<(), JsError> {
        self.separator
            .submit(&spec, &wav)
            .map(|_| ())
            .map_err(|e| JsError::new(&e.to_string()))
    }

    /// Finishes separation and analyzes the song.
    #[wasm_bindgen(js_name = intoSong)]
    pub fn into_song(self) -> Result<WasmSong, JsError> {
        let stems = self
            .separator
            .into_stems()
            .map_err(|e| JsError::new(&e.to_string()))?;
        Ok(WasmSong::new(self.name, SongAudio::Separated { stems }))
    }
}

/// Collects tracks for a [`WasmPlayer`] (the worklet receives them one by one).
#[wasm_bindgen]
#[derive(Default)]
pub struct PlayerBuilder {
    tracks: Vec<PlayerTrack>,
    reference: Vec<Note>,
}

#[wasm_bindgen]
impl PlayerBuilder {
    #[wasm_bindgen(constructor)]
    pub fn new() -> PlayerBuilder {
        PlayerBuilder::default()
    }

    #[wasm_bindgen(js_name = pushTrack)]
    pub fn push_track(
        &mut self,
        left: Vec<f32>,
        right: Vec<f32>,
        mix: JsValue,
    ) -> Result<(), JsError> {
        self.tracks.push(PlayerTrack {
            left: left.into(),
            right: right.into(),
            mix: from_js(mix)?,
        });
        Ok(())
    }

    /// The song's reference melody (`Note[]`), for follow-melody auto-tune.
    #[wasm_bindgen(js_name = setReference)]
    pub fn set_reference(&mut self, notes: JsValue) -> Result<(), JsError> {
        self.reference = from_js(notes)?;
        Ok(())
    }

    /// `sample_rate`: the AudioContext's rate (cues are synthesized at it).
    pub fn build(self, sample_rate: u32) -> WasmPlayer {
        WasmPlayer {
            player: Player::new(self.tracks, sample_rate, self.reference),
            left: vec![0.0; MAX_BLOCK],
            right: vec![0.0; MAX_BLOCK],
            input: vec![0.0; MAX_BLOCK],
        }
    }
}

/// Real-time player for the AudioWorklet. JS reads the output through views on wasm memory
/// at [`WasmPlayer::left_ptr`] / [`WasmPlayer::right_ptr`], and writes the microphone block
/// for the monitor at [`WasmPlayer::input_ptr`], so a block never allocates.
#[wasm_bindgen]
pub struct WasmPlayer {
    player: Player,
    left: Vec<f32>,
    right: Vec<f32>,
    input: Vec<f32>,
}

#[wasm_bindgen]
impl WasmPlayer {
    /// Applies a `PlayerCommand`; illegal transitions are returned as errors.
    pub fn apply(&mut self, command: JsValue) -> Result<(), JsError> {
        let command: PlayerCommand = from_js(command)?;
        self.player
            .apply(command)
            .map_err(|e| JsError::new(&e.to_string()))
    }

    /// Adds a track (a take) without reallocating; errors when the reserve is used up.
    #[wasm_bindgen(js_name = addTrack)]
    pub fn add_track(
        &mut self,
        left: Vec<f32>,
        right: Vec<f32>,
        mix: JsValue,
    ) -> Result<(), JsError> {
        self.player
            .add_track(PlayerTrack {
                left: left.into(),
                right: right.into(),
                mix: from_js(mix)?,
            })
            .map_err(|_| JsError::new("too many takes in one session"))
    }

    /// Renders `frames` (≤ 1024) samples into the output buffers. `with_input`: the
    /// microphone block was written at [`WasmPlayer::input_ptr`] (for the monitor).
    pub fn render(&mut self, frames: usize, with_input: bool) {
        let n = frames.min(MAX_BLOCK);
        let input = if with_input { &self.input[..n] } else { &[] };
        self.player
            .render(&mut self.left[..n], &mut self.right[..n], input);
    }

    #[wasm_bindgen(js_name = inputPtr)]
    pub fn input_ptr(&mut self) -> *mut f32 {
        self.input.as_mut_ptr()
    }

    /// Delay of the tuned monitor, in frames.
    #[wasm_bindgen(js_name = monitorLatency)]
    pub fn monitor_latency(&self) -> usize {
        self.player.monitor_latency()
    }

    #[wasm_bindgen(js_name = leftPtr)]
    pub fn left_ptr(&self) -> *const f32 {
        self.left.as_ptr()
    }

    #[wasm_bindgen(js_name = rightPtr)]
    pub fn right_ptr(&self) -> *const f32 {
        self.right.as_ptr()
    }

    /// Current `Transport`.
    pub fn transport(&self) -> Result<JsValue, JsError> {
        to_js(&self.player.transport())
    }
}

/// Live karaoke pitch of a capture (main thread, fed from the capture ring's drains).
#[wasm_bindgen]
pub struct WasmLive {
    session: LiveSession,
}

#[wasm_bindgen]
impl WasmLive {
    /// Feeds capture samples (in order); returns a `LiveUpdate`.
    pub fn push(&mut self, samples: &[f32]) -> Result<JsValue, JsError> {
        to_js(&self.session.push(samples))
    }
}
