//! Offline noise suppression with DeepFilterNet3 (AGENTS.md §8, Produce → Clean up): the
//! vendored `deep-filter-rt` runs the model with tract on the CPU. The model manager keeps
//! upstream's `DeepFilterNet3_onnx.tar.gz` (manifest id [`MODEL`]); its four members are read
//! straight from the archive. The model runs at 48 kHz, so takes are resampled there and back.

use anyhow::{anyhow, Context, Result};
use deep_filter_rt::tract::{DfParams, DfTract, RuntimeParams};
use engine_core::resample::resample;
use ndarray::{ArrayView2, ArrayViewMut2};
use std::io::Read;
use std::path::Path;

/// The manifest id of DeepFilterNet3.
pub const MODEL: &str = "deepfilternet3";
/// The manifest file name of its archive.
pub const ARCHIVE: &str = "DeepFilterNet3_onnx.tar.gz";
/// The model's sample rate.
pub const MODEL_RATE: u32 = 48_000;
/// How far noise is pulled down at most (dB). Full suppression sounds processed on singing;
/// this keeps a little air, as DeepFilterNet's own tools suggest for music.
pub const ATTENUATION_LIMIT_DB: f32 = 30.0;

/// The four model files from the archive, by name.
fn members(archive: &Path) -> Result<DfParams> {
    let file = std::fs::File::open(archive).with_context(|| archive.display().to_string())?;
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let (mut config, mut enc, mut erb, mut df) = (None, None, None, None);
    for entry in tar.entries()? {
        let mut entry = entry?;
        let name = entry
            .path()?
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        let slot = match name.as_deref() {
            Some("config.ini") => &mut config,
            Some("enc.onnx") => &mut enc,
            Some("erb_dec.onnx") => &mut erb,
            Some("df_dec.onnx") => &mut df,
            _ => continue,
        };
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry.read_to_end(&mut bytes)?;
        *slot = Some(bytes);
    }
    let missing = |what: &str| anyhow!("{ARCHIVE} has no {what}");
    DfParams::from_bytes(
        &config.ok_or_else(|| missing("config.ini"))?,
        enc.ok_or_else(|| missing("enc.onnx"))?,
        erb.ok_or_else(|| missing("erb_dec.onnx"))?,
        df.ok_or_else(|| missing("df_dec.onnx"))?,
    )
}

pub struct Denoiser {
    df: DfTract,
}

impl Denoiser {
    pub fn load(archive: &Path) -> Result<Self> {
        let params = RuntimeParams::default_with_ch(1).with_atten_lim(ATTENUATION_LIMIT_DB);
        Ok(Self {
            df: DfTract::new(members(archive)?, &params)?,
        })
    }

    /// `signal` (mono, `sample_rate`) with its noise suppressed; same length and timing.
    pub fn run(&mut self, signal: &[f32], sample_rate: u32) -> Result<Vec<f32>> {
        let hop = self.df.hop_size;
        let delay = self.df.delay();
        let mut x = resample(&[signal], sample_rate, MODEL_RATE)?.remove(0);
        let len = x.len();
        // Room for the model's delay, then whole hops.
        x.resize((len + delay).div_ceil(hop) * hop, 0.0);
        let mut y = vec![0.0f32; x.len()];
        for (i, o) in x.chunks_exact(hop).zip(y.chunks_exact_mut(hop)) {
            let i = ArrayView2::from_shape((1, hop), i)?;
            let o = ArrayViewMut2::from_shape((1, hop), o)?;
            self.df.process(i, o)?;
        }
        let y = &y[delay..delay + len];
        let mut out = resample(&[y], MODEL_RATE, sample_rate)?.remove(0);
        out.resize(signal.len(), 0.0);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::TAU;

    fn archive() -> Option<std::path::PathBuf> {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../models/weights")
            .join(ARCHIVE);
        p.exists().then_some(p)
    }

    /// Speech-like: a voiced 140 Hz buzz with harmonics, syllables 4 per second with pauses.
    fn speech(sr: u32, seconds: f32) -> Vec<f32> {
        let n = (sr as f32 * seconds) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / sr as f32;
                let f0 = 140.0 + 20.0 * (TAU * 0.5 * t).sin();
                let syllable = (TAU * 2.0 * t).sin().max(0.0).powf(0.5);
                let voiced: f32 = (1..12)
                    .map(|h| (TAU * f0 * h as f32 * t).sin() / h as f32)
                    .sum();
                0.25 * syllable * voiced
            })
            .collect()
    }

    /// White noise from a fixed LCG.
    fn noise(n: usize, amp: f32) -> Vec<f32> {
        let mut s = 0x1234_5678u32;
        (0..n)
            .map(|_| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                amp * ((s >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0)
            })
            .collect()
    }

    fn snr_db(clean: &[f32], test: &[f32]) -> f32 {
        let s: f32 = clean.iter().map(|v| v * v).sum();
        let e: f32 = clean.iter().zip(test).map(|(a, b)| (a - b) * (a - b)).sum();
        10.0 * (s / e).log10()
    }

    #[test]
    fn denoising_raises_snr_on_speech_in_noise() {
        let Some(path) = archive() else {
            eprintln!("skipped: {ARCHIVE} not in models/weights");
            return;
        };
        let sr = 44_100;
        let clean = speech(sr, 4.0);
        let noisy: Vec<f32> = clean
            .iter()
            .zip(noise(clean.len(), 0.08))
            .map(|(a, b)| a + b)
            .collect();
        let mut d = Denoiser::load(&path).unwrap();
        let start = std::time::Instant::now();
        let out = d.run(&noisy, sr).unwrap();
        let took = start.elapsed().as_secs_f32();
        assert_eq!(out.len(), noisy.len());
        // Skip the first half second (the model's norms settle).
        let skip = sr as usize / 2;
        let before = snr_db(&clean[skip..], &noisy[skip..]);
        let after = snr_db(&clean[skip..], &out[skip..]);
        eprintln!(
            "denoise: SNR {before:.1} dB -> {after:.1} dB (+{:.1} dB), {:.2}x real time",
            after - before,
            took / 4.0
        );
        assert!(after > before + 5.0, "{before} -> {after}");
    }
}
