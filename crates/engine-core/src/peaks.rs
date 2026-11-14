//! Waveform overview for the timeline.

/// Samples per peak pair: ~43 pairs per second at 44.1 kHz.
pub const SAMPLES_PER_PEAK: usize = 1024;

/// Flattened `[min, max, min, max, …]` per block of [`SAMPLES_PER_PEAK`] samples.
pub fn peaks(samples: &[f32]) -> Vec<f32> {
    samples
        .chunks(SAMPLES_PER_PEAK)
        .flat_map(|c| {
            let (lo, hi) = c
                .iter()
                .fold((f32::MAX, f32::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
            [lo, hi]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn min_max_per_block() {
        let mut x = vec![0.0f32; 2048];
        x[5] = -0.5;
        x[1500] = 0.75;
        assert_eq!(super::peaks(&x), vec![-0.5, 0.0, 0.0, 0.75]);
    }
}
