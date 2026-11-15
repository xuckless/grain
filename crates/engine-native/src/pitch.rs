//! Pitch tracking for references and takes: RMVPE through ONNX Runtime when its weights are
//! installed, the built-in YIN otherwise (AGENTS.md §4.4, §5).

use crate::separate::session;
use anyhow::{Context, Result};
use engine_core::project::{PitchTracker, TrackedPitch};
use engine_core::resample::resample;
use engine_ml::rmvpe::{self, BINS, N_MELS};
use ort::session::Session;
use ort::value::Tensor;
use std::path::{Path, PathBuf};

pub const RMVPE_FILE: &str = "rmvpe.onnx";

pub struct Rmvpe {
    session: Session,
}

impl Rmvpe {
    pub fn load(onnx: &Path) -> Result<Self> {
        Ok(Self {
            session: session(onnx)?,
        })
    }

    /// Pitch contour (10 ms frames) of mono `signal` at `sample_rate`.
    pub fn track(&mut self, signal: &[f32], sample_rate: u32) -> Result<TrackedPitch> {
        let audio = resample(&[signal], sample_rate, rmvpe::SAMPLE_RATE)?.remove(0);
        let input = rmvpe::features(&audio);
        let outputs = self.session.run(ort::inputs![
            "input" => Tensor::from_array(([1usize, N_MELS, input.padded], input.mel))?,
        ])?;
        let (shape, salience) = outputs["output"].try_extract_tensor::<f32>()?;
        anyhow::ensure!(
            salience.len() == input.padded * BINS,
            "unexpected RMVPE output shape {shape:?}"
        );
        Ok(TrackedPitch {
            tracker: PitchTracker::Rmvpe,
            track: rmvpe::decode(salience, input.frames),
        })
    }
}

/// The host's pitch trackers. RMVPE is optional and loaded on first use.
pub enum Pitch {
    NotLoaded(PathBuf),
    Loaded(Rmvpe),
    /// No RMVPE weights: everything is tracked with YIN.
    YinOnly,
}

impl Pitch {
    /// RMVPE from the first of `dirs` that holds [`RMVPE_FILE`], else YIN only.
    pub fn find(dirs: &[PathBuf]) -> Self {
        match dirs.iter().map(|d| d.join(RMVPE_FILE)).find(|p| p.exists()) {
            Some(path) => Pitch::NotLoaded(path),
            None => Pitch::YinOnly,
        }
    }

    /// The tracker new songs are analysed with.
    pub fn preferred(&self) -> PitchTracker {
        match self {
            Pitch::NotLoaded(_) | Pitch::Loaded(_) => PitchTracker::Rmvpe,
            Pitch::YinOnly => PitchTracker::Yin,
        }
    }

    /// Tracks `signal` with `tracker` (a project's reference tracker, so takes match it).
    pub fn track(
        &mut self,
        tracker: PitchTracker,
        signal: &[f32],
        sample_rate: u32,
    ) -> Result<TrackedPitch> {
        match tracker {
            PitchTracker::Yin => Ok(TrackedPitch::yin(signal, sample_rate)),
            PitchTracker::Rmvpe => self.rmvpe()?.track(signal, sample_rate),
        }
    }

    fn rmvpe(&mut self) -> Result<&mut Rmvpe> {
        if let Pitch::NotLoaded(path) = self {
            *self = Pitch::Loaded(Rmvpe::load(path).context("loading RMVPE")?);
        }
        match self {
            Pitch::Loaded(r) => Ok(r),
            Pitch::YinOnly => {
                anyhow::bail!("this song was analysed with RMVPE, which isn't installed")
            }
            Pitch::NotLoaded(_) => unreachable!("loaded above"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::note::hz_to_midi;
    use engine_core::pitch::PitchFrame;

    /// Needs the weights: `cargo test -p engine-native -- --ignored rmvpe`.
    #[test]
    #[ignore = "needs models/weights/rmvpe.onnx"]
    fn rmvpe_tracks_sines_and_vibrato() {
        let onnx = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models/weights/rmvpe.onnx");
        let mut rmvpe = Rmvpe::load(&onnx).unwrap();
        let sr = 44_100u32;
        let tau = std::f64::consts::TAU;
        // (base Hz, vibrato extent in cents, rate Hz)
        for (base, extent, rate) in [(110.0, 0.0, 0.0), (261.63, 0.0, 0.0), (440.0, 50.0, 5.5)] {
            let mut phase = 0.0f64;
            let signal: Vec<f32> = (0..sr as usize * 3)
                .map(|i| {
                    let t = i as f64 / sr as f64;
                    let hz = base * 2f64.powf(extent * (tau * rate * t).sin() / 1200.0);
                    phase += tau * hz / sr as f64;
                    // A few harmonics, like a voice.
                    (0.3 * phase.sin() + 0.15 * (2.0 * phase).sin() + 0.08 * (3.0 * phase).sin())
                        as f32
                })
                .collect();
            let track = rmvpe.track(&signal, sr).unwrap().track;
            let errors: Vec<f32> = track
                .frames
                .iter()
                .enumerate()
                .skip(10)
                .take(track.frames.len() - 20)
                .map(|(i, f)| {
                    let t = i as f64 * track.hop_seconds;
                    let truth = base * 2f64.powf(extent * (tau * rate * t).sin() / 1200.0);
                    match f {
                        PitchFrame::Voiced { midi } => 100.0 * (midi - hz_to_midi(truth as f32)),
                        PitchFrame::Unvoiced => f32::INFINITY,
                    }
                })
                .collect();
            let within = errors.iter().filter(|e| e.abs() <= 20.0).count();
            let ratio = within as f32 / errors.len() as f32;
            let mean = errors
                .iter()
                .filter(|e| e.is_finite())
                .map(|e| e.abs())
                .sum::<f32>()
                / errors.len() as f32;
            eprintln!(
                "{base} Hz vibrato {extent}: {:.1}% within 20 cents, mean |err| {mean:.1}",
                100.0 * ratio
            );
            assert!(ratio > 0.95, "{base} Hz: {ratio}");
        }
    }
}
