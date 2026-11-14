//! In-memory audio buffers.

/// Planar stereo audio. Mono sources are stored with both channels equal.
#[derive(Debug, Clone, PartialEq)]
pub struct StereoAudio {
    pub sample_rate: u32,
    pub left: Vec<f32>,
    pub right: Vec<f32>,
}

impl StereoAudio {
    pub fn len(&self) -> usize {
        self.left.len()
    }

    pub fn is_empty(&self) -> bool {
        self.left.is_empty()
    }

    pub fn duration_seconds(&self) -> f64 {
        self.len() as f64 / self.sample_rate as f64
    }

    pub fn to_mono(&self) -> Vec<f32> {
        self.left
            .iter()
            .zip(&self.right)
            .map(|(l, r)| 0.5 * (l + r))
            .collect()
    }
}
