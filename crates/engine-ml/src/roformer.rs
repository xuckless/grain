//! Mel-Band RoFormer vocal separation, host side (AGENTS.md §4.2). The ONNX graph maps a
//! spectrogram to a complex mask (see `scripts/export_melband_roformer.py`):
//!
//! ```text
//! input  spec [1, 2F, T, 2]   STFT of both channels, row 2f+c = bin f of channel c, (re, im)
//! output mask [1, 2F, T, 2]   complex mask, same layout
//! vocals = ISTFT(spec * mask) per channel
//! ```
//!
//! Chunking follows the reference `demix_track`: reflect-pad the song by the chunk border
//! (`chunk - step`), run `chunk_samples` chunks every `step_samples`, cross-fade with linear fades
//! of a tenth of a chunk (none at the song's two ends), divide by the summed window. The
//! reference steps half a chunk; a step of `chunk - chunk / 10` overlaps chunks by exactly one
//! fade (fades sum to 1) and needs ~45% fewer model runs.

use crate::all_finite;
use engine_core::audio::StereoAudio;
use engine_core::stft::{reflect_pad, Complex, Spectrum, Stft};
use serde::Deserialize;

/// The `<model>.json` sidecar next to the graph.
#[derive(Debug, Clone, Deserialize)]
pub struct RoformerSpec {
    pub name: String,
    pub samplerate: u32,
    pub audio_channels: usize,
    pub chunk_samples: usize,
    pub step_samples: usize,
    pub nfft: usize,
    pub hop: usize,
    pub freq_bins: usize,
    pub frames: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum RoformerError {
    #[error("invalid model sidecar: {0}")]
    Spec(String),
    #[error("{0} is not allowed in state {1:?}")]
    State(&'static str, VocalSeparatorState),
    #[error("model output has {got} values, expected {expected}")]
    OutputShape { got: usize, expected: usize },
    #[error("model produced non-finite output on chunk {0}")]
    NonFinite(usize),
}

impl RoformerSpec {
    pub fn parse(json: &str) -> Result<Self, RoformerError> {
        let spec: Self =
            serde_json::from_str(json).map_err(|e| RoformerError::Spec(e.to_string()))?;
        let valid = spec.audio_channels == 2
            && spec.step_samples > 0
            && spec.step_samples <= spec.chunk_samples - spec.chunk_samples / 10
            && spec.freq_bins == spec.nfft / 2 + 1
            && spec.frames == spec.chunk_samples / spec.hop + 1;
        if !valid {
            return Err(RoformerError::Spec(format!("unsupported layout: {spec:?}")));
        }
        Ok(spec)
    }

    /// Element count of `spec` and `mask` (batch 1).
    pub fn tensor_len(&self) -> usize {
        2 * self.freq_bins * self.frames * 2
    }

    fn step(&self) -> usize {
        self.step_samples
    }

    fn border(&self) -> usize {
        self.chunk_samples - self.step()
    }

    /// Reflect padding added on each side of a song of `len` samples (only when it is long
    /// enough to mirror).
    fn pad(&self, len: usize) -> usize {
        let border = self.border();
        if border > 0 && len > 2 * border {
            border
        } else {
            0
        }
    }

    /// Chunks needed for a song of `len` samples.
    pub fn chunk_count(&self, len: usize) -> usize {
        (len + 2 * self.pad(len)).div_ceil(self.step()).max(1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VocalSeparatorState {
    /// Waiting for the mask of chunk `index` (0-based) of `count`.
    AwaitingChunk {
        index: usize,
        count: usize,
    },
    Complete,
}

pub struct VocalSeparator {
    spec: RoformerSpec,
    stft: Stft,
    /// The song, reflect-padded by `pad` on both sides.
    input: [Vec<f32>; 2],
    pad: usize,
    output: [Vec<f32>; 2],
    weight_sum: Vec<f32>,
    fade: Vec<f32>,
    state: VocalSeparatorState,
}

impl VocalSeparator {
    /// `mix` must be at `spec.samplerate`.
    pub fn new(spec: RoformerSpec, mix: &StereoAudio) -> Self {
        assert_eq!(
            mix.sample_rate, spec.samplerate,
            "resample before separating"
        );
        let pad = spec.pad(mix.len());
        let input = [&mix.left, &mix.right].map(|x| reflect_pad(x, pad, pad));
        let len = input[0].len();
        let count = spec.chunk_count(mix.len());
        let fade_len = spec.chunk_samples / 10;
        let fade = (0..fade_len)
            .map(|i| i as f32 / (fade_len.max(2) - 1) as f32)
            .collect();
        Self {
            stft: Stft::new(spec.nfft, spec.hop),
            output: [vec![0.0; len], vec![0.0; len]],
            weight_sum: vec![0.0; len],
            input,
            pad,
            fade,
            state: VocalSeparatorState::AwaitingChunk { index: 0, count },
            spec,
        }
    }

    pub fn state(&self) -> VocalSeparatorState {
        self.state
    }

    /// The `spec` input for the chunk the separator is waiting for.
    pub fn inputs(&self) -> Result<Vec<f32>, RoformerError> {
        let VocalSeparatorState::AwaitingChunk { index, .. } = self.state else {
            return Err(RoformerError::State("inputs", self.state));
        };
        let spectra = self.chunk_spectra(index);
        let (bins, frames) = (self.spec.freq_bins, self.spec.frames);
        let mut spec = vec![0.0f32; self.spec.tensor_len()];
        for (c, channel) in spectra.iter().enumerate() {
            for (t, frame) in channel.iter().enumerate() {
                for (f, z) in frame.iter().enumerate() {
                    let at = (((2 * f + c) * frames) + t) * 2;
                    spec[at] = z.re;
                    spec[at + 1] = z.im;
                }
            }
        }
        debug_assert_eq!(spectra[0].len(), frames);
        debug_assert_eq!(spectra[0][0].len(), bins);
        Ok(spec)
    }

    /// Accepts the mask for the current chunk and advances.
    pub fn submit(&mut self, mask: &[f32]) -> Result<VocalSeparatorState, RoformerError> {
        let VocalSeparatorState::AwaitingChunk { index, count } = self.state else {
            return Err(RoformerError::State("submit", self.state));
        };
        let expected = self.spec.tensor_len();
        if mask.len() != expected {
            return Err(RoformerError::OutputShape {
                got: mask.len(),
                expected,
            });
        }
        if !all_finite(mask) {
            return Err(RoformerError::NonFinite(index));
        }
        let frames = self.spec.frames;
        let (start, valid) = self.chunk_range(index);
        let window = self.window(index);
        for (c, spectra) in self.chunk_spectra(index).into_iter().enumerate() {
            let masked: Vec<Spectrum> = spectra
                .into_iter()
                .enumerate()
                .map(|(t, frame)| {
                    frame
                        .into_iter()
                        .enumerate()
                        .map(|(f, z)| {
                            let at = (((2 * f + c) * frames) + t) * 2;
                            z * Complex::new(mask[at], mask[at + 1])
                        })
                        .collect()
                })
                .collect();
            let audio = self.stft.inverse(&masked, self.spec.chunk_samples);
            let out = &mut self.output[c][start..start + valid];
            for ((o, a), w) in out.iter_mut().zip(&audio).zip(&window) {
                *o += a * w;
            }
        }
        for (s, w) in self.weight_sum[start..start + valid]
            .iter_mut()
            .zip(&window)
        {
            *s += w;
        }
        self.state = if index + 1 < count {
            VocalSeparatorState::AwaitingChunk {
                index: index + 1,
                count,
            }
        } else {
            VocalSeparatorState::Complete
        };
        Ok(self.state)
    }

    /// The separated vocals, the length of the original mix.
    pub fn into_vocals(self) -> Result<StereoAudio, RoformerError> {
        if self.state != VocalSeparatorState::Complete {
            return Err(RoformerError::State("into_vocals", self.state));
        }
        let (pad, len) = (self.pad, self.weight_sum.len() - 2 * self.pad);
        let weight = &self.weight_sum;
        let [left, right] = self.output.map(|x| {
            (pad..pad + len)
                .map(|i| x[i] / weight[i].max(1e-8))
                .collect::<Vec<f32>>()
        });
        Ok(StereoAudio {
            sample_rate: self.spec.samplerate,
            left,
            right,
        })
    }

    /// Start of chunk `index` in the padded song and how many of its samples are real.
    fn chunk_range(&self, index: usize) -> (usize, usize) {
        let start = index * self.spec.step();
        (
            start,
            self.spec.chunk_samples.min(self.weight_sum.len() - start),
        )
    }

    /// STFT of both channels of a chunk. A short final chunk is reflect-padded when more than
    /// half full, zero-padded otherwise (as the reference does).
    fn chunk_spectra(&self, index: usize) -> [Vec<Spectrum>; 2] {
        let (start, valid) = self.chunk_range(index);
        let chunk = self.spec.chunk_samples;
        self.input.clone().map(|x| {
            let part = &x[start..start + valid];
            let padded = if valid == chunk {
                part.to_vec()
            } else if valid > chunk / 2 + 1 {
                reflect_pad(part, 0, chunk - valid)
            } else {
                let mut p = part.to_vec();
                p.resize(chunk, 0.0);
                p
            };
            self.stft.forward(&padded)
        })
    }

    /// Linear fade-in/out of a tenth of a chunk; no fade-in on the first chunk and no fade-out
    /// on chunks reaching the song's end.
    fn window(&self, index: usize) -> Vec<f32> {
        let chunk = self.spec.chunk_samples;
        let n = self.fade.len();
        let mut w = vec![1.0f32; chunk];
        if index > 0 {
            w[..n].copy_from_slice(&self.fade);
        }
        if index * self.spec.step() + chunk < self.weight_sum.len() {
            for (i, f) in self.fade.iter().rev().enumerate() {
                w[chunk - n + i] *= f;
            }
        }
        w
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small_spec() -> RoformerSpec {
        RoformerSpec {
            name: "test".into(),
            samplerate: 8000,
            audio_channels: 2,
            chunk_samples: 4000,
            step_samples: 2000,
            nfft: 256,
            hop: 50,
            freq_bins: 129,
            frames: 4000 / 50 + 1,
        }
    }

    /// An all-pass mask (1 + 0i) must give the input back through padding, chunking, STFT,
    /// cross-fades and trimming.
    #[test]
    fn unit_mask_reconstructs_the_mix() {
        let spec = small_spec();
        let len = 13_217;
        let mix = StereoAudio {
            sample_rate: 8000,
            left: (0..len).map(|i| 0.3 * (i as f32 * 0.05).sin()).collect(),
            right: (0..len)
                .map(|i| 0.2 * (i as f32 * 0.031).cos() + 0.05)
                .collect(),
        };
        let mut sep = VocalSeparator::new(spec.clone(), &mix);
        let unit: Vec<f32> = (0..spec.tensor_len())
            .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
            .collect();
        while let VocalSeparatorState::AwaitingChunk { .. } = sep.state() {
            sep.inputs().unwrap();
            sep.submit(&unit).unwrap();
        }
        let out = sep.into_vocals().unwrap();
        assert_eq!(out.len(), len);
        for i in (0..len).step_by(37) {
            assert!((out.left[i] - mix.left[i]).abs() < 1e-3, "left {i}");
            assert!((out.right[i] - mix.right[i]).abs() < 1e-3, "right {i}");
        }
    }

    #[test]
    fn fade_step_reconstructs_the_mix() {
        let spec = RoformerSpec {
            step_samples: 3600,
            ..small_spec()
        };
        let len = 21_001;
        let mix = StereoAudio {
            sample_rate: 8000,
            left: (0..len).map(|i| 0.3 * (i as f32 * 0.05).sin()).collect(),
            right: (0..len).map(|i| 0.2 * (i as f32 * 0.031).cos()).collect(),
        };
        let mut sep = VocalSeparator::new(spec.clone(), &mix);
        let unit: Vec<f32> = (0..spec.tensor_len())
            .map(|i| if i % 2 == 0 { 1.0 } else { 0.0 })
            .collect();
        let mut runs = 0;
        while let VocalSeparatorState::AwaitingChunk { .. } = sep.state() {
            sep.submit(&unit).unwrap();
            runs += 1;
        }
        assert_eq!(runs, spec.chunk_count(len));
        let out = sep.into_vocals().unwrap();
        for i in (0..len).step_by(31) {
            assert!((out.left[i] - mix.left[i]).abs() < 1e-3, "left {i}");
        }
    }

    #[test]
    fn rejects_bad_masks_and_specs() {
        let spec = small_spec();
        let mix = StereoAudio {
            sample_rate: 8000,
            left: vec![0.0; 100],
            right: vec![0.0; 100],
        };
        let mut sep = VocalSeparator::new(spec.clone(), &mix);
        assert!(matches!(
            sep.submit(&[1.0]),
            Err(RoformerError::OutputShape { .. })
        ));
        let mut nan = vec![0.0f32; spec.tensor_len()];
        nan[3] = f32::NAN;
        assert!(matches!(sep.submit(&nan), Err(RoformerError::NonFinite(0))));
        assert!(RoformerSpec::parse(r#"{"name":"x","samplerate":44100,"audio_channels":2,"chunk_samples":352800,"step_samples":176400,"nfft":2048,"hop":441,"freq_bins":1000,"frames":801}"#).is_err());
    }
}
