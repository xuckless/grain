//! HTDemucs host processing for graphs exported with the STFT outside the network
//! (HRSadeghi/demucs-onnx layout, AGENTS.md §4.2):
//!
//! ```text
//! inputs   mix  [1, 2, S]          time-domain segment
//!          mag  [1, 4, F, T]       complex-as-channels spectrogram (2c = re, 2c+1 = im)
//! outputs  spec [1, N, 4, F, T]    per-source spectrogram
//!          wav  [1, N, 2, S]       per-source time-branch waveform
//! stem = wav + ISTFT(spec)
//! ```
//!
//! [`Separator`] is an explicit state machine so synchronous (ort) and asynchronous
//! (onnxruntime-web) hosts drive it the same way.

use engine_core::audio::StereoAudio;
use engine_core::project::Stem;
use engine_core::stft::{reflect_pad, Complex, Spectrum, Stft};
use serde::Deserialize;

/// Fraction of each segment shared with its neighbor.
const OVERLAP: f64 = 0.25;

/// The `<model>.json` sidecar next to each graph.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelSpec {
    pub name: String,
    pub sources: Vec<String>,
    pub samplerate: u32,
    pub audio_channels: usize,
    pub segment_samples: usize,
    pub nfft: usize,
    pub hop: usize,
    pub freq_bins: usize,
    pub frames: usize,
    pub cac: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum DemucsError {
    #[error("invalid model sidecar: {0}")]
    Spec(String),
    #[error("{0} is not allowed in state {1:?}")]
    State(&'static str, SeparatorState),
    #[error("model output has {got} values, expected {expected}")]
    OutputShape { got: usize, expected: usize },
}

impl ModelSpec {
    pub fn parse(json: &str) -> Result<Self, DemucsError> {
        let spec: Self =
            serde_json::from_str(json).map_err(|e| DemucsError::Spec(e.to_string()))?;
        let valid = spec.cac
            && spec.audio_channels == 2
            && spec.hop * 4 == spec.nfft
            && spec.freq_bins == spec.nfft / 2
            && spec.frames == spec.segment_samples.div_ceil(spec.hop);
        if !valid {
            return Err(DemucsError::Spec(format!("unsupported layout: {spec:?}")));
        }
        spec.stems()?;
        Ok(spec)
    }

    pub fn stems(&self) -> Result<Vec<Stem>, DemucsError> {
        self.sources
            .iter()
            .map(|s| match s.as_str() {
                "drums" => Ok(Stem::Drums),
                "bass" => Ok(Stem::Bass),
                "other" => Ok(Stem::Other),
                "vocals" => Ok(Stem::Vocals),
                "guitar" => Ok(Stem::Guitar),
                "piano" => Ok(Stem::Piano),
                other => Err(DemucsError::Spec(format!("unknown source {other}"))),
            })
            .collect()
    }

    /// Element counts of `mix`, `mag`, `spec`, `wav` (batch 1).
    pub fn mix_len(&self) -> usize {
        2 * self.segment_samples
    }
    pub fn mag_len(&self) -> usize {
        4 * self.freq_bins * self.frames
    }
    pub fn spec_len(&self) -> usize {
        self.sources.len() * self.mag_len()
    }
    pub fn wav_len(&self) -> usize {
        self.sources.len() * self.mix_len()
    }
}

/// One segment's model inputs, row-major, batch 1.
pub struct SegmentInputs {
    pub mix: Vec<f32>,
    pub mag: Vec<f32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeparatorState {
    /// Waiting for the model outputs of segment `index` (0-based) of `count`.
    AwaitingSegment {
        index: usize,
        count: usize,
    },
    Complete,
}

pub struct Separator {
    spec: ModelSpec,
    stft: Stft,
    /// Normalized input, planar.
    input: [Vec<f32>; 2],
    /// Weighted sum of segment outputs per stem and channel.
    output: Vec<[Vec<f32>; 2]>,
    weight_sum: Vec<f32>,
    weight: Vec<f32>,
    offsets: Vec<usize>,
    mean: f32,
    std: f32,
    state: SeparatorState,
}

impl Separator {
    /// `mix` must be at `spec.samplerate`.
    pub fn new(spec: ModelSpec, mix: &StereoAudio) -> Self {
        assert_eq!(
            mix.sample_rate, spec.samplerate,
            "resample before separating"
        );
        // Whole-track normalization on the mono reference, as demucs' `separate` does.
        let mono = mix.to_mono();
        let n = mono.len().max(2) as f32;
        let mean = mono.iter().sum::<f32>() / n;
        let std = (mono.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / (n - 1.0))
            .sqrt()
            .max(1e-8);
        let norm = |x: &[f32]| x.iter().map(|v| (v - mean) / std).collect::<Vec<f32>>();

        let segment = spec.segment_samples;
        let stride = ((1.0 - OVERLAP) * segment as f64) as usize;
        let offsets: Vec<usize> = (0..mix.len().max(1)).step_by(stride).collect();
        // Triangular cross-fade weights (demucs `apply_model`, transition_power = 1).
        let half = segment / 2;
        let weight: Vec<f32> = (1..=half)
            .chain((1..=segment - half).rev())
            .map(|w| w as f32 / half.max(segment - half) as f32)
            .collect();
        let len = mix.len();
        let count = offsets.len();
        Self {
            stft: Stft::new(spec.nfft, spec.hop),
            input: [norm(&mix.left), norm(&mix.right)],
            output: (0..spec.sources.len())
                .map(|_| [vec![0.0; len], vec![0.0; len]])
                .collect(),
            weight_sum: vec![0.0; len],
            weight,
            offsets,
            mean,
            std,
            state: SeparatorState::AwaitingSegment { index: 0, count },
            spec,
        }
    }

    pub fn state(&self) -> SeparatorState {
        self.state
    }

    /// Model inputs for the segment the separator is waiting for.
    pub fn inputs(&self) -> Result<SegmentInputs, DemucsError> {
        let SeparatorState::AwaitingSegment { index, .. } = self.state else {
            return Err(DemucsError::State("inputs", self.state));
        };
        let chunk = self.chunk(self.offsets[index]);
        let mut mix = Vec::with_capacity(self.spec.mix_len());
        mix.extend_from_slice(&chunk[0]);
        mix.extend_from_slice(&chunk[1]);
        let mut mag = vec![0.0f32; self.spec.mag_len()];
        let (f_bins, frames) = (self.spec.freq_bins, self.spec.frames);
        for (c, channel) in chunk.iter().enumerate() {
            for (t, frame) in self.spectrogram(channel).iter().enumerate() {
                for (f, z) in frame.iter().take(f_bins).enumerate() {
                    mag[((2 * c) * f_bins + f) * frames + t] = z.re;
                    mag[((2 * c + 1) * f_bins + f) * frames + t] = z.im;
                }
            }
        }
        Ok(SegmentInputs { mix, mag })
    }

    /// Accepts the model outputs for the current segment and advances.
    pub fn submit(
        &mut self,
        spec_out: &[f32],
        wav_out: &[f32],
    ) -> Result<SeparatorState, DemucsError> {
        let SeparatorState::AwaitingSegment { index, count } = self.state else {
            return Err(DemucsError::State("submit", self.state));
        };
        for (got, expected) in [
            (spec_out.len(), self.spec.spec_len()),
            (wav_out.len(), self.spec.wav_len()),
        ] {
            if got != expected {
                return Err(DemucsError::OutputShape { got, expected });
            }
        }
        let offset = self.offsets[index];
        let segment = self.spec.segment_samples;
        let valid = segment.min(self.weight_sum.len() - offset);
        let (f_bins, frames) = (self.spec.freq_bins, self.spec.frames);
        for s in 0..self.spec.sources.len() {
            for c in 0..2 {
                let plane = |p: usize, f: usize, t: usize| {
                    spec_out[(((s * 4) + p) * f_bins + f) * frames + t]
                };
                let spectrum: Vec<Spectrum> = (0..frames)
                    .map(|t| {
                        (0..f_bins)
                            .map(|f| Complex::new(plane(2 * c, f, t), plane(2 * c + 1, f, t)))
                            .collect()
                    })
                    .collect();
                let freq = self.inverse_spectrogram(&spectrum);
                let wav = &wav_out[(s * 2 + c) * segment..(s * 2 + c + 1) * segment];
                let out = &mut self.output[s][c];
                for i in 0..valid {
                    out[offset + i] += self.weight[i] * (wav[i] + freq[i]);
                }
            }
        }
        for i in 0..valid {
            self.weight_sum[offset + i] += self.weight[i];
        }
        self.state = if index + 1 < count {
            SeparatorState::AwaitingSegment {
                index: index + 1,
                count,
            }
        } else {
            SeparatorState::Complete
        };
        Ok(self.state)
    }

    /// The separated stems, in the model's source order.
    pub fn into_stems(self) -> Result<Vec<(Stem, StereoAudio)>, DemucsError> {
        if self.state != SeparatorState::Complete {
            return Err(DemucsError::State("into_stems", self.state));
        }
        let stems = self.spec.stems()?;
        let (mean, std, sr) = (self.mean, self.std, self.spec.samplerate);
        let weight_sum = self.weight_sum;
        let denorm = |x: Vec<f32>| -> Vec<f32> {
            x.into_iter()
                .zip(&weight_sum)
                .map(|(v, w)| v / w.max(1e-8) * std + mean)
                .collect()
        };
        Ok(stems
            .into_iter()
            .zip(self.output)
            .map(|(stem, [left, right])| {
                (
                    stem,
                    StereoAudio {
                        sample_rate: sr,
                        left: denorm(left),
                        right: denorm(right),
                    },
                )
            })
            .collect())
    }

    /// Segment starting at `offset`, zero-padded past the end of the song.
    fn chunk(&self, offset: usize) -> [Vec<f32>; 2] {
        let segment = self.spec.segment_samples;
        self.input.clone().map(|x| {
            let mut c = vec![0.0f32; segment];
            let end = (offset + segment).min(x.len());
            c[..end - offset].copy_from_slice(&x[offset..end]);
            c
        })
    }

    /// `HTDemucs._spec`: re-pad so frames align with the time branch, drop the Nyquist bin
    /// and two edge frames, normalized STFT.
    fn spectrogram(&self, x: &[f32]) -> Vec<Spectrum> {
        let hop = self.spec.hop;
        let le = x.len().div_ceil(hop);
        let pad = hop / 2 * 3;
        let padded = reflect_pad(x, pad, pad + le * hop - x.len());
        let scale = 1.0 / (self.spec.nfft as f32).sqrt();
        self.stft
            .forward(&padded)
            .into_iter()
            .skip(2)
            .take(le)
            .map(|frame| {
                frame
                    .into_iter()
                    .take(self.spec.freq_bins)
                    .map(|z| z * scale)
                    .collect()
            })
            .collect()
    }

    /// `HTDemucs._ispec`: restore the Nyquist bin and edge frames, normalized ISTFT, trim.
    fn inverse_spectrogram(&self, z: &[Spectrum]) -> Vec<f32> {
        let hop = self.spec.hop;
        let length = self.spec.segment_samples;
        let pad = hop / 2 * 3;
        let le = hop * length.div_ceil(hop) + 2 * pad;
        let bins = self.stft.bins();
        let scale = (self.spec.nfft as f32).sqrt();
        let silent: Spectrum = vec![Default::default(); bins];
        let frames: Vec<Spectrum> = std::iter::repeat_n(silent.clone(), 2)
            .chain(z.iter().map(|frame| {
                let mut f: Spectrum = frame.iter().map(|v| v * scale).collect();
                f.resize(bins, Default::default());
                f
            }))
            .chain(std::iter::repeat_n(silent, 2))
            .collect();
        let x = self.stft.inverse(&frames, le);
        x[pad..pad + length].to_vec()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn small_spec(sources: usize) -> ModelSpec {
        let names = ["drums", "bass", "other", "vocals"];
        ModelSpec {
            name: "test".into(),
            sources: names[..sources].iter().map(|s| s.to_string()).collect(),
            samplerate: 8000,
            audio_channels: 2,
            segment_samples: 4000,
            nfft: 256,
            hop: 64,
            freq_bins: 128,
            frames: 4000usize.div_ceil(64),
            cac: true,
        }
    }

    /// A fake model that routes the spectrogram to source 0 and the waveform to source 1,
    /// scaled so the sum of sources must reconstruct the normalized input.
    fn fake_model(spec: &ModelSpec, inputs: &SegmentInputs) -> (Vec<f32>, Vec<f32>) {
        let mut spec_out = vec![0.0f32; spec.spec_len()];
        spec_out[..spec.mag_len()]
            .iter_mut()
            .zip(&inputs.mag)
            .for_each(|(o, m)| *o = 0.5 * m);
        let mut wav_out = vec![0.0f32; spec.wav_len()];
        wav_out[spec.mix_len()..2 * spec.mix_len()]
            .iter_mut()
            .zip(&inputs.mix)
            .for_each(|(o, m)| *o = 0.5 * m);
        (spec_out, wav_out)
    }

    #[test]
    fn fake_model_reconstructs_mix_through_chunking_and_istft() {
        let spec = small_spec(2);
        let len = 11_000; // not a multiple of the stride: exercises the zero-padded tail
        let mix = StereoAudio {
            sample_rate: 8000,
            left: (0..len).map(|i| 0.3 * (i as f32 * 0.05).sin()).collect(),
            right: (0..len)
                .map(|i| 0.2 * (i as f32 * 0.031).cos() + 0.05)
                .collect(),
        };
        let mut sep = Separator::new(spec.clone(), &mix);
        let mut segments = 0;
        while let SeparatorState::AwaitingSegment { .. } = sep.state() {
            let inputs = sep.inputs().unwrap();
            let (s, w) = fake_model(&spec, &inputs);
            sep.submit(&s, &w).unwrap();
            segments += 1;
        }
        assert_eq!(segments, 4); // offsets 0, 3000, 6000, 9000
        assert!(sep.inputs().is_err());
        let stems = sep.into_stems().unwrap();
        let (_, a) = &stems[0];
        let (_, b) = &stems[1];
        let mean = mix.to_mono().iter().sum::<f32>() / len as f32;
        // Each fake source carries half the normalized signal; denormalizing adds the mean to both,
        // so the sum of sources is the input plus one extra mean.
        for i in (100..len - 100).step_by(97) {
            let recon = a.left[i] + b.left[i] - mean;
            assert!(
                (recon - mix.left[i]).abs() < 1e-3,
                "sample {i}: {recon} vs {}",
                mix.left[i]
            );
            let recon = a.right[i] + b.right[i] - mean;
            assert!((recon - mix.right[i]).abs() < 1e-3);
        }
    }

    #[test]
    fn rejects_wrong_output_shapes_and_bad_specs() {
        let spec = small_spec(2);
        let mix = StereoAudio {
            sample_rate: 8000,
            left: vec![0.0; 100],
            right: vec![0.0; 100],
        };
        let mut sep = Separator::new(spec, &mix);
        assert!(matches!(
            sep.submit(&[0.0], &[0.0]),
            Err(DemucsError::OutputShape { .. })
        ));
        assert!(ModelSpec::parse(r#"{"name":"x","sources":["kazoo"],"samplerate":44100,"audio_channels":2,"segment_samples":343980,"nfft":4096,"hop":1024,"freq_bins":2048,"frames":336,"cac":true}"#).is_err());
    }
}
