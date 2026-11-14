//! Web bindings. Two consumers:
//! - the main thread: [`WasmSeparator`] → [`WasmSong`] (import, analysis, scoring);
//! - the AudioWorklet: [`WasmPlayer`] (real-time mixing; no allocation per block).

use engine_core::audio::StereoAudio;
use engine_core::player::{Player, PlayerCommand, PlayerTrack};
use engine_core::project::{self, Project, SongAudio, SongSource, TrackedPitch};
use engine_core::score::ScoreSettings;
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

/// An imported song: its audio plus the analyzed project.
#[wasm_bindgen]
pub struct WasmSong {
    project: Project,
    audio: SongAudio,
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
        }
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

    /// Scores a take (same sample rate as the song) and returns a `ScoreReport`.
    #[wasm_bindgen(js_name = scoreTake)]
    pub fn score_take(
        &self,
        left: Vec<f32>,
        right: Vec<f32>,
        settings: JsValue,
    ) -> Result<JsValue, JsError> {
        let settings: ScoreSettings = from_js(settings)?;
        let take = stereo(left, right, self.project.sample_rate)?;
        let take = TrackedPitch::yin(&take.to_mono(), self.project.sample_rate);
        let report = project::score_take(&self.project, &take, settings)
            .map_err(|e| JsError::new(&e.to_string()))?;
        to_js(&report)
    }
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

    pub fn build(self) -> WasmPlayer {
        WasmPlayer {
            player: Player::new(self.tracks),
            left: vec![0.0; MAX_BLOCK],
            right: vec![0.0; MAX_BLOCK],
        }
    }
}

/// Real-time player for the AudioWorklet. JS reads the output through views on wasm memory
/// at [`WasmPlayer::left_ptr`] / [`WasmPlayer::right_ptr`], so a block never allocates.
#[wasm_bindgen]
pub struct WasmPlayer {
    player: Player,
    left: Vec<f32>,
    right: Vec<f32>,
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

    /// Renders `frames` (≤ 1024) samples into the output buffers.
    pub fn render(&mut self, frames: usize) {
        let n = frames.min(MAX_BLOCK);
        self.player
            .render(&mut self.left[..n], &mut self.right[..n]);
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
