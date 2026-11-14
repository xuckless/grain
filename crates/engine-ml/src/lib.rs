//! Host-side model processing. Inference runs wherever the host runs it (ort natively,
//! onnxruntime-web in the browser); this crate prepares inputs and assembles outputs.

pub mod demucs;
pub mod rmvpe;
pub mod roformer;

/// Whether every model output value is finite (some execution providers emit NaN/inf).
pub(crate) fn all_finite(values: &[f32]) -> bool {
    values.iter().all(|v| v.is_finite())
}
