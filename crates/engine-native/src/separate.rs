//! HTDemucs separation through ONNX Runtime (AGENTS.md §4.2).

use anyhow::{Context, Result};
use engine_core::audio::StereoAudio;
use engine_core::project::Stem;
use engine_ml::demucs::{ModelSpec, Separator, SeparatorState};
use ort::session::Session;
use ort::value::Tensor;
use std::path::Path;

pub struct Demucs {
    session: Session,
    spec: ModelSpec,
}

impl Demucs {
    /// Loads `<name>.onnx` and its `<name>.json` sidecar.
    pub fn load(onnx: &Path) -> Result<Self> {
        let sidecar = onnx.with_extension("json");
        let spec = ModelSpec::parse(
            &std::fs::read_to_string(&sidecar)
                .with_context(|| format!("reading {}", sidecar.display()))?,
        )?;
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
        let session = builder
            .commit_from_file(onnx)
            .with_context(|| format!("loading {}", onnx.display()))?;
        Ok(Self { session, spec })
    }

    /// Model id used in the stem cache key.
    pub fn id(&self) -> &str {
        &self.spec.name
    }

    pub fn sample_rate(&self) -> u32 {
        self.spec.samplerate
    }

    /// Separates `mix` (at the model's sample rate). `progress(done, total)` runs per segment.
    pub fn separate(
        &mut self,
        mix: &StereoAudio,
        mut progress: impl FnMut(usize, usize),
    ) -> Result<Vec<(Stem, StereoAudio)>> {
        let spec = &self.spec;
        let (s, f, t, n) = (
            spec.segment_samples,
            spec.freq_bins,
            spec.frames,
            spec.sources.len(),
        );
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
            debug_assert_eq!(spec_out.len(), n * 4 * f * t);
            separator.submit(spec_out, wav_out)?;
        }
        Ok(separator.into_stems()?)
    }
}
