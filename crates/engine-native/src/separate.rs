//! Stem separation through ONNX Runtime (AGENTS.md §4.2): HTDemucs alone, or the vocal cascade
//! (Mel-Band RoFormer vocals, then HTDemucs on the instrumental).

use anyhow::{Context, Result};
use engine_core::audio::StereoAudio;
use engine_core::project::{SeparationQuality, Stem};
use engine_ml::demucs::{ModelSpec, Separator, SeparatorState};
use engine_ml::roformer::{RoformerSpec, VocalSeparator, VocalSeparatorState};
use ort::session::Session;
use ort::value::Tensor;
use std::path::{Path, PathBuf};

/// Separation weights a host looks for (`models/weights` layout, AGENTS.md §4.2).
pub const DEMUCS_FILE: &str = "htdemucs_6s.onnx";
pub const VOCALS_FILE: &str = "melband_roformer_vocals.onnx";
/// Id of the vocal model (its sidecar `name`), known without loading it.
const VOCALS_ID: &str = "melband_roformer_vocals";

/// An ONNX Runtime session with the build's execution provider (CPU unless a feature says
/// otherwise; CoreML produced NaN on these models, AGENTS.md §2).
pub(crate) fn session(onnx: &Path) -> Result<Session> {
    let builder = Session::builder()?;
    #[cfg(feature = "coreml")]
    let builder = builder.with_execution_providers([
        ort::execution_providers::CoreMLExecutionProvider::default().build(),
    ])?;
    #[cfg(feature = "directml")]
    let builder = builder.with_execution_providers([
        ort::execution_providers::DirectMLExecutionProvider::default().build(),
    ])?;
    #[cfg(feature = "cuda")]
    let builder = builder.with_execution_providers([
        ort::execution_providers::CUDAExecutionProvider::default().build(),
    ])?;
    builder
        .commit_from_file(onnx)
        .with_context(|| format!("loading {}", onnx.display()))
}

fn sidecar(onnx: &Path) -> Result<String> {
    let path = onnx.with_extension("json");
    std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))
}

pub struct Demucs {
    session: Session,
    spec: ModelSpec,
}

impl Demucs {
    /// Loads `<name>.onnx` and its `<name>.json` sidecar.
    pub fn load(onnx: &Path) -> Result<Self> {
        Ok(Self {
            spec: ModelSpec::parse(&sidecar(onnx)?)?,
            session: session(onnx)?,
        })
    }

    /// Segments [`Demucs::separate`] runs for a song of `len` samples.
    pub fn segment_count(&self, len: usize) -> usize {
        self.spec.segment_count(len)
    }

    /// Separates `mix` (at the model's sample rate). `progress(done, total)` runs per segment.
    pub fn separate(
        &mut self,
        mix: &StereoAudio,
        mut progress: impl FnMut(usize, usize),
    ) -> Result<Vec<(Stem, StereoAudio)>> {
        let spec = &self.spec;
        let (s, f, t) = (spec.segment_samples, spec.freq_bins, spec.frames);
        let mut separator = Separator::new(spec.clone(), mix);
        while let SeparatorState::AwaitingSegment { index, count } = separator.state() {
            progress(index, count);
            let inputs = separator.inputs()?;
            let outputs = self.session.run(ort::inputs![
                "mix" => Tensor::from_array(([1usize, 2, s], inputs.mix))?,
                "mag" => Tensor::from_array(([1usize, 4, f, t], inputs.mag))?,
            ])?;
            let (_, spec_out) = outputs["spec"].try_extract_tensor::<f32>()?;
            let (_, wav_out) = outputs["wav"].try_extract_tensor::<f32>()?;
            separator.submit(spec_out, wav_out)?;
        }
        Ok(separator.into_stems()?)
    }
}

pub struct Roformer {
    session: Session,
    spec: RoformerSpec,
}

impl Roformer {
    /// Loads `<name>.onnx` and its `<name>.json` sidecar.
    pub fn load(onnx: &Path) -> Result<Self> {
        Ok(Self {
            spec: RoformerSpec::parse(&sidecar(onnx)?)?,
            session: session(onnx)?,
        })
    }

    /// Extracts the vocals of `mix`. `progress(done, total)` runs per chunk.
    pub fn vocals(
        &mut self,
        mix: &StereoAudio,
        mut progress: impl FnMut(usize, usize),
    ) -> Result<StereoAudio> {
        let shape = [1usize, 2 * self.spec.freq_bins, self.spec.frames, 2];
        let mut separator = VocalSeparator::new(self.spec.clone(), mix);
        while let VocalSeparatorState::AwaitingChunk { index, count } = separator.state() {
            progress(index, count);
            let spec = separator.inputs()?;
            let outputs = self
                .session
                .run(ort::inputs!["spec" => Tensor::from_array((shape, spec))?])?;
            let (_, mask) = outputs["mask"].try_extract_tensor::<f32>()?;
            separator.submit(mask)?;
        }
        Ok(separator.into_vocals()?)
    }
}

/// The vocal model of the cascade: optional, and loaded on first use (it holds ~1 GB).
pub enum VocalModel {
    NotLoaded(PathBuf),
    Loaded(Roformer),
    /// [`VOCALS_FILE`] is not installed: only [`SeparationQuality::Fast`] is offered.
    Missing,
}

/// The separation models a host has loaded.
pub struct Separators {
    rest: Demucs,
    vocals: VocalModel,
}

impl Separators {
    /// Loads the models in the first of `dirs` holding [`DEMUCS_FILE`] (plus [`VOCALS_FILE`]
    /// from the same directory, if present).
    pub fn find(dirs: &[PathBuf]) -> Result<Option<Self>> {
        let Some(dir) = dirs.iter().find(|d| d.join(DEMUCS_FILE).exists()) else {
            return Ok(None);
        };
        let vocals = dir.join(VOCALS_FILE);
        Ok(Some(Self {
            rest: Demucs::load(&dir.join(DEMUCS_FILE))?,
            vocals: if vocals.exists() {
                VocalModel::NotLoaded(vocals)
            } else {
                VocalModel::Missing
            },
        }))
    }

    /// Whether `quality` can run, judged from the files alone (nothing is loaded).
    pub fn offers(dirs: &[PathBuf], quality: SeparationQuality) -> bool {
        let file = match quality {
            SeparationQuality::Fast => DEMUCS_FILE,
            SeparationQuality::BestVocals => VOCALS_FILE,
        };
        dirs.iter()
            .any(|d| d.join(DEMUCS_FILE).exists() && d.join(file).exists())
    }

    /// Model id used in the stem cache key.
    pub fn id(&self, quality: SeparationQuality) -> Result<String> {
        Ok(match (quality, &self.vocals) {
            (SeparationQuality::Fast, _) => self.rest.spec.name.clone(),
            (SeparationQuality::BestVocals, VocalModel::Missing) => {
                anyhow::bail!("the vocal model ({VOCALS_FILE}) is not installed")
            }
            (SeparationQuality::BestVocals, _) => format!("{VOCALS_ID}+{}", self.rest.spec.name),
        })
    }

    /// Separates `mix`. `progress(done, total)` counts model runs across all stages. The
    /// cascade takes RoFormer's vocals, runs HTDemucs on the rest and folds HTDemucs's vocal
    /// output (what RoFormer left behind) into Other, so the stems still sum to the mix.
    pub fn separate(
        &mut self,
        quality: SeparationQuality,
        mix: &StereoAudio,
        mut progress: impl FnMut(usize, usize),
    ) -> Result<Vec<(Stem, StereoAudio)>> {
        if quality == SeparationQuality::Fast {
            return self.rest.separate(mix, progress);
        }
        let vocals = self.vocals.load()?;
        let first = vocals.spec.chunk_count(mix.len());
        let total = first + self.rest.segment_count(mix.len());
        let voice = vocals.vocals(mix, |i, _| progress(i, total))?;
        let instrumental = zip_with(mix, &voice, |m, v| m - v);
        let mut stems = self
            .rest
            .separate(&instrumental, |i, _| progress(first + i, total))?;
        if let Some(i) = stems.iter().position(|(s, _)| *s == Stem::Vocals) {
            let (_, residue) = stems.remove(i);
            if let Some((_, other)) = stems.iter_mut().find(|(s, _)| *s == Stem::Other) {
                *other = zip_with(other, &residue, |o, r| o + r);
            }
        }
        stems.insert(0, (Stem::Vocals, voice));
        Ok(stems)
    }
}

impl VocalModel {
    /// The model, loading it on first use.
    fn load(&mut self) -> Result<&mut Roformer> {
        if let VocalModel::NotLoaded(path) = self {
            *self = VocalModel::Loaded(Roformer::load(path)?);
        }
        match self {
            VocalModel::Loaded(v) => Ok(v),
            VocalModel::Missing => {
                anyhow::bail!("the vocal model ({VOCALS_FILE}) is not installed")
            }
            VocalModel::NotLoaded(_) => unreachable!("loaded above"),
        }
    }
}

fn zip_with(a: &StereoAudio, b: &StereoAudio, f: impl Fn(f32, f32) -> f32) -> StereoAudio {
    let channel = |x: &[f32], y: &[f32]| x.iter().zip(y).map(|(p, q)| f(*p, *q)).collect();
    StereoAudio {
        sample_rate: a.sample_rate,
        left: channel(&a.left, &b.left),
        right: channel(&a.right, &b.right),
    }
}
