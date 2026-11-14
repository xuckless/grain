//! Host-side model processing. Inference runs wherever the host runs it (ort natively,
//! onnxruntime-web in the browser); this crate prepares inputs and assembles outputs.

pub mod demucs;
