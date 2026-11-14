//! Sample-rate conversion (rubato FFT resampler), shared by decoding, playback and models that
//! run at their own rate (RMVPE at 16 kHz).

use rubato::{FftFixedIn, Resampler};

pub use rubato::ResamplerConstructionError as ResampleError;

const CHUNK: usize = 4096;

/// Resamples equal-length channels from `from` to `to` Hz. The output keeps the input's
/// duration (`len * to / from` samples) with the filter delay removed.
pub fn resample(channels: &[&[f32]], from: u32, to: u32) -> Result<Vec<Vec<f32>>, ResampleError> {
    if from == to {
        return Ok(channels.iter().map(|c| c.to_vec()).collect());
    }
    let n = channels.len();
    let len = channels.first().map_or(0, |c| c.len());
    let mut resampler = FftFixedIn::<f32>::new(from as usize, to as usize, CHUNK, 2, n)?;
    let delay = resampler.output_delay();
    let expected = (len as u64 * to as u64 / from as u64) as usize;
    let mut out = vec![Vec::with_capacity(expected + delay + CHUNK); n];
    let mut chunk = vec![vec![0.0f32; CHUNK]; n];
    let mut pos = 0;
    // Feed the signal, then silence until the delayed tail is out.
    while pos < len || out[0].len() < expected + delay {
        let end = (pos + CHUNK).min(len);
        for (c, src) in chunk.iter_mut().zip(channels) {
            c.fill(0.0);
            if pos < end {
                c[..end - pos].copy_from_slice(&src[pos..end]);
            }
        }
        pos = end;
        let produced = resampler
            .process(&chunk, None)
            .expect("chunk sizes come from the resampler");
        for (o, p) in out.iter_mut().zip(produced) {
            o.extend(p);
        }
    }
    Ok(out
        .into_iter()
        .map(|o| o[delay..delay + expected].to_vec())
        .collect())
}
