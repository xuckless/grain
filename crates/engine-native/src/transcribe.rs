//! MIDI layers on desktop (AGENTS.md §4.3): basic-pitch through ONNX Runtime for pitched tracks,
//! the built-in classifier for drums.

use crate::separate::session;
use anyhow::{ensure, Result};
use engine_core::drums;
use engine_core::midi::{self, MidiNote, Transcription, TranscriptionMethod};
use engine_core::project::TrackSource;
use engine_ml::basic_pitch::{self, FRAMES, KEYS, WINDOW};
use ort::session::Session;
use ort::value::Tensor;
use std::path::Path;

/// Windows per inference call (bounds memory on long songs).
const BATCH: usize = 8;

pub struct BasicPitch {
    session: Session,
}

impl BasicPitch {
    pub fn load(onnx: &Path) -> Result<Self> {
        Ok(Self {
            session: session(onnx)?,
        })
    }

    /// Notes in mono `signal` at `sample_rate`.
    pub fn transcribe(&mut self, signal: &[f32], sample_rate: u32) -> Result<Vec<MidiNote>> {
        let input = basic_pitch::input(signal, sample_rate)?;
        let mut note = Vec::with_capacity(input.count * FRAMES * KEYS);
        let mut onset = Vec::with_capacity(input.count * FRAMES * KEYS);
        for chunk in input.windows.chunks(BATCH * WINDOW) {
            let n = chunk.len() / WINDOW;
            let outputs = self.session.run(ort::inputs![
                basic_pitch::INPUT => Tensor::from_array(([n, WINDOW, 1usize], chunk.to_vec()))?,
            ])?;
            for (name, out) in [
                (basic_pitch::NOTE_OUTPUT, &mut note),
                (basic_pitch::ONSET_OUTPUT, &mut onset),
            ] {
                let (_, values) = outputs[name].try_extract_tensor::<f32>()?;
                ensure!(
                    values.len() == n * FRAMES * KEYS,
                    "unexpected basic-pitch output size"
                );
                out.extend_from_slice(values);
            }
        }
        Ok(basic_pitch::notes(
            &note,
            &onset,
            input.count,
            input.samples,
        ))
    }
}

/// Transcribes track `source` (its mono audio `signal`). Pitched tracks need basic-pitch's
/// weights at `model` (`None`: not installed).
pub fn transcribe(
    source: TrackSource,
    signal: &[f32],
    sample_rate: u32,
    model: Option<&Path>,
) -> Result<Transcription, Missing> {
    let method = midi::method_for(source);
    let notes = match method {
        TranscriptionMethod::DrumClassifier => drums::transcribe(signal, sample_rate),
        TranscriptionMethod::BasicPitch => {
            let path = model.ok_or(Missing::Model)?;
            BasicPitch::load(path)
                .and_then(|mut m| m.transcribe(signal, sample_rate))
                .map_err(Missing::Failed)?
        }
    };
    Ok(Transcription {
        source,
        method,
        notes,
    })
}

/// Why a transcription didn't happen.
#[derive(Debug, thiserror::Error)]
pub enum Missing {
    #[error("basic-pitch isn't downloaded")]
    Model,
    #[error(transparent)]
    Failed(#[from] anyhow::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: u32 = 22_050;

    /// The repository's basic-pitch weights, when fetched (`scripts/fetch_models.py`).
    fn weights() -> Option<std::path::PathBuf> {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../models/weights")
            .join(basic_pitch::MODEL_FILE);
        p.exists().then_some(p)
    }

    /// A plucked synth note (harmonics rolling off as 1/k², like a soft string).
    fn pluck(midi: u8, seconds: f64) -> Vec<f32> {
        let hz = 440.0 * 2f32.powf((f32::from(midi) - 69.0) / 12.0);
        (0..(seconds * f64::from(SR)) as usize)
            .map(|i| {
                let t = i as f32 / SR as f32;
                let env = (t * 200.0).min(1.0) * (-t * 2.5).exp();
                (1..=5)
                    .map(|k| {
                        0.4 / (k * k) as f32
                            * (2.0 * std::f32::consts::PI * hz * k as f32 * t).sin()
                    })
                    .sum::<f32>()
                    * env
            })
            .collect()
    }

    /// Onset-and-pitch F1 (onsets within 50 ms, the mir_eval convention).
    fn f1(want: &[(f64, u8)], got: &[MidiNote]) -> f64 {
        let mut used = vec![false; got.len()];
        let mut hits = 0;
        for &(t, p) in want {
            if let Some(j) = (0..got.len())
                .find(|&j| !used[j] && got[j].pitch == p && (got[j].start - t).abs() < 0.05)
            {
                used[j] = true;
                hits += 1;
            }
        }
        let precision = hits as f64 / got.len().max(1) as f64;
        let recall = hits as f64 / want.len() as f64;
        if hits == 0 {
            0.0
        } else {
            2.0 * precision * recall / (precision + recall)
        }
    }

    #[test]
    fn basic_pitch_transcribes_a_synthetic_melody() {
        let Some(path) = weights() else {
            eprintln!("skipped: basic-pitch weights not fetched");
            return;
        };
        // A C major line across several windows (so the overlap stitching is exercised).
        let melody: [u8; 10] = [60, 62, 64, 65, 67, 69, 71, 72, 67, 64];
        let mut signal = vec![0.0f32; SR as usize / 4];
        let mut want = Vec::new();
        for &m in &melody {
            want.push((signal.len() as f64 / f64::from(SR), m));
            signal.extend(pluck(m, 0.5));
        }
        signal.extend(vec![0.0; SR as usize / 2]);
        let notes = BasicPitch::load(&path)
            .unwrap()
            .transcribe(&signal, SR)
            .unwrap();
        let score = f1(&want, &notes);
        eprintln!(
            "basic-pitch note F1 {score:.3} ({} notes for {})",
            notes.len(),
            want.len()
        );
        assert!(score >= 0.8, "F1 {score}: {notes:?}");
    }
}
