//! basic-pitch audio-to-MIDI (AGENTS.md §4.3), host side: windowing and the note decoding of
//! Spotify's `basic_pitch/inference.py` and `note_creation.py` (Apache-2.0), around the published
//! `nmp.onnx` (registered as `basic_pitch` in the manifest):
//!
//! ```text
//! input  serving_default_input_2:0  [N, 43844, 1]  mono 22.05 kHz windows
//! output StatefulPartitionedCall:1  [N, 172, 88]   note (frame) activations
//!        StatefulPartitionedCall:2  [N, 172, 88]   onset activations
//!        StatefulPartitionedCall:0  [N, 172, 264]  contour (unused here)
//! ```
//!
//! Hosts run [`input`]'s windows through the model (in batches of their choice, in order) and
//! hand the concatenated note and onset outputs to [`notes`].

use engine_core::midi::MidiNote;
use engine_core::resample::{resample, ResampleError};

pub const MODEL_FILE: &str = "basic_pitch.onnx";
pub const INPUT: &str = "serving_default_input_2:0";
pub const NOTE_OUTPUT: &str = "StatefulPartitionedCall:1";
pub const ONSET_OUTPUT: &str = "StatefulPartitionedCall:2";

pub const SAMPLE_RATE: u32 = 22_050;
const FFT_HOP: usize = 256;
/// Samples per model window (2 s minus one hop).
pub const WINDOW: usize = 43_844;
/// Frames per window, and keys (A0 up).
pub const FRAMES: usize = 172;
pub const KEYS: usize = 88;
const OVERLAP_FRAMES: usize = 30;
const OVERLAP: usize = OVERLAP_FRAMES * FFT_HOP;
const HOP: usize = WINDOW - OVERLAP;
const MIDI_OFFSET: u8 = 21;

/// Decoding settings (basic-pitch's defaults).
const ONSET_THRESHOLD: f32 = 0.5;
const FRAME_THRESHOLD: f32 = 0.3;
/// 127.7 ms at 86 frames/s.
const MIN_NOTE_FRAMES: usize = 11;
const ENERGY_TOLERANCE: usize = 11;
const ALIGNMENT_OFFSET: f64 = 0.0018;

/// The model's input for one track.
pub struct Input {
    /// `count` windows of [`WINDOW`] samples, back to back.
    pub windows: Vec<f32>,
    pub count: usize,
    /// Length of the track at [`SAMPLE_RATE`] (for trimming the output).
    pub samples: usize,
}

/// Windows of mono `signal` (at `sample_rate`) for the model (`get_audio_input`).
pub fn input(signal: &[f32], sample_rate: u32) -> Result<Input, ResampleError> {
    let audio = resample(&[signal], sample_rate, SAMPLE_RATE)?.remove(0);
    let mut padded = vec![0.0f32; OVERLAP / 2];
    padded.extend_from_slice(&audio);
    let count = padded.len().div_ceil(HOP).max(1);
    let mut windows = vec![0.0f32; count * WINDOW];
    for k in 0..count {
        let a = k * HOP;
        let b = (a + WINDOW).min(padded.len());
        if a < b {
            windows[k * WINDOW..k * WINDOW + (b - a)].copy_from_slice(&padded[a..b]);
        }
    }
    Ok(Input {
        windows,
        count,
        samples: audio.len(),
    })
}

/// The per-window outputs (`[count, FRAMES, KEYS]`) joined into one `[frames, KEYS]` matrix with
/// the overlaps removed and trimmed to the audio (`unwrap_output`).
fn unwrap(output: &[f32], count: usize, samples: usize) -> Vec<f32> {
    let keep = FRAMES - OVERLAP_FRAMES;
    let half = OVERLAP_FRAMES / 2;
    let mut out = Vec::with_capacity(count * keep * KEYS);
    for k in 0..count {
        let w = &output[k * FRAMES * KEYS..(k + 1) * FRAMES * KEYS];
        out.extend_from_slice(&w[half * KEYS..(FRAMES - half) * KEYS]);
    }
    let frames = ((samples as f64 / HOP as f64) * keep as f64) as usize;
    out.truncate(frames.min(count * keep) * KEYS);
    out
}

/// Song time of output frame `i` (`model_frames_to_time`).
fn frame_time(i: usize) -> f64 {
    let hop = FFT_HOP as f64 / f64::from(SAMPLE_RATE);
    let window_offset = hop * (FRAMES as f64 - WINDOW as f64 / FFT_HOP as f64) + ALIGNMENT_OFFSET;
    i as f64 * hop - window_offset * (i / FRAMES) as f64
}

/// Notes from the model's note and onset outputs (`[count, FRAMES, KEYS]` each).
pub fn notes(note: &[f32], onset: &[f32], count: usize, samples: usize) -> Vec<MidiNote> {
    let frames = unwrap(note, count, samples);
    let onsets = unwrap(onset, count, samples);
    decode(&frames, &onsets)
        .into_iter()
        .map(|(a, b, key, amp)| MidiNote {
            start: frame_time(a),
            end: frame_time(b),
            pitch: key as u8 + MIDI_OFFSET,
            velocity: (127.0 * amp).round().clamp(1.0, 127.0) as u8,
        })
        .collect()
}

/// `output_to_notes_polyphonic` with inferred onsets and the melodia trick: `(start frame, end
/// frame, key, amplitude)` per note, from `[n, KEYS]` frame and onset activations.
fn decode(frames: &[f32], onsets: &[f32]) -> Vec<(usize, usize, usize, f32)> {
    let n = frames.len() / KEYS;
    if n < 3 {
        return Vec::new();
    }
    let at = |t: usize, k: usize| t * KEYS + k;
    let onsets = infer_onsets(onsets, frames, n);
    let mut remaining = frames.to_vec();
    let mut events = Vec::new();

    // Onset peaks over the threshold, latest first.
    let mut peaks: Vec<(usize, usize)> = Vec::new();
    for t in 1..n - 1 {
        for k in 0..KEYS {
            let v = onsets[at(t, k)];
            if v >= ONSET_THRESHOLD && v > onsets[at(t - 1, k)] && v > onsets[at(t + 1, k)] {
                peaks.push((t, k));
            }
        }
    }
    peaks.sort_by(|a, b| b.cmp(a));
    for (start, k) in peaks {
        if start >= n - 1 {
            continue;
        }
        let (mut i, mut below) = (start + 1, 0);
        while i < n - 1 && below < ENERGY_TOLERANCE {
            below = if remaining[at(i, k)] < FRAME_THRESHOLD {
                below + 1
            } else {
                0
            };
            i += 1;
        }
        i -= below;
        if i - start <= MIN_NOTE_FRAMES {
            continue;
        }
        for t in start..i {
            clear(&mut remaining, t, k);
        }
        events.push((start, i, k, mean(frames, start, i, k)));
    }

    // Melodia trick: grow notes out from the strongest remaining frames. `remaining` only ever
    // loses energy, so walking the cells strongest-first and skipping cleared ones visits them in
    // the order of repeated arg-maxes.
    let mut cells: Vec<usize> = (0..remaining.len())
        .filter(|&c| remaining[c] > FRAME_THRESHOLD)
        .collect();
    cells.sort_by(|&a, &b| remaining[b].total_cmp(&remaining[a]));
    for c in cells {
        if remaining[c] <= FRAME_THRESHOLD {
            continue;
        }
        let (mid, k) = (c / KEYS, c % KEYS);
        remaining[c] = 0.0;
        let (mut i, mut below) = (mid + 1, 0);
        while i < n - 1 && below < ENERGY_TOLERANCE {
            below = if remaining[at(i, k)] < FRAME_THRESHOLD {
                below + 1
            } else {
                0
            };
            clear(&mut remaining, i, k);
            i += 1;
        }
        let end = i - 1 - below;
        let (mut i, mut below) = (mid as isize - 1, 0);
        while i > 0 && below < ENERGY_TOLERANCE {
            below = if remaining[at(i as usize, k)] < FRAME_THRESHOLD {
                below + 1
            } else {
                0
            };
            clear(&mut remaining, i as usize, k);
            i -= 1;
        }
        let start = (i + 1 + below as isize).max(0) as usize;
        if end <= start || end - start <= MIN_NOTE_FRAMES {
            continue;
        }
        events.push((start, end, k, mean(frames, start, end, k)));
    }
    events
}

/// Zeroes key `k` and its neighbours at frame `t`.
fn clear(m: &mut [f32], t: usize, k: usize) {
    for j in k.saturating_sub(1)..=(k + 1).min(KEYS - 1) {
        m[t * KEYS + j] = 0.0;
    }
}

fn mean(frames: &[f32], a: usize, b: usize, k: usize) -> f32 {
    (a..b).map(|t| frames[t * KEYS + k]).sum::<f32>() / (b - a).max(1) as f32
}

/// Onsets plus large rises in the frame activations (`get_infered_onsets`, two-frame differences).
fn infer_onsets(onsets: &[f32], frames: &[f32], n: usize) -> Vec<f32> {
    let at = |t: usize, k: usize| t * KEYS + k;
    let mut diff = vec![0.0f32; frames.len()];
    for t in 2..n {
        for k in 0..KEYS {
            let d1 = frames[at(t, k)] - frames[at(t - 1, k)];
            let d2 = frames[at(t, k)] - frames[at(t - 2, k)];
            diff[at(t, k)] = d1.min(d2).max(0.0);
        }
    }
    let max_onset = onsets.iter().copied().fold(0.0f32, f32::max);
    let max_diff = diff.iter().copied().fold(0.0f32, f32::max);
    let scale = if max_diff > 0.0 {
        max_onset / max_diff
    } else {
        0.0
    };
    onsets
        .iter()
        .zip(&diff)
        .map(|(o, d)| o.max(d * scale))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_overlap_and_cover_the_audio() {
        let signal = vec![0.5f32; SAMPLE_RATE as usize * 5];
        let input = input(&signal, SAMPLE_RATE).unwrap();
        assert_eq!(input.samples, signal.len());
        assert_eq!(input.windows.len(), input.count * WINDOW);
        // Window 0 starts with half an overlap of silence; window 1 starts a hop later.
        assert_eq!(input.windows[OVERLAP / 2 - 1], 0.0);
        assert_eq!(input.windows[OVERLAP / 2], 0.5);
        assert_eq!(input.windows[WINDOW], 0.5);
        assert!(input.count * HOP >= signal.len());
    }

    /// Activations for one window: key `k` on from frame `a` to `b`, with an onset at `a`.
    fn activations(count: usize, notes: &[(usize, usize, usize)]) -> (Vec<f32>, Vec<f32>) {
        let mut frame = vec![0.0f32; count * FRAMES * KEYS];
        let mut onset = vec![0.0f32; count * FRAMES * KEYS];
        for &(k, a, b) in notes {
            for t in a..b {
                frame[t * KEYS + k] = 0.8;
            }
            onset[a * KEYS + k] = 0.9;
        }
        (frame, onset)
    }

    #[test]
    fn decodes_held_notes_from_activations() {
        // Frames 15.. are kept (the overlap is trimmed), so note at window frame 40 is output
        // frame 25.
        let (frame, onset) = activations(1, &[(39, 40, 90), (43, 100, 140)]);
        let samples = HOP; // one window's worth
        let mut notes = notes(&frame, &onset, 1, samples);
        notes.sort_by(|a, b| a.start.total_cmp(&b.start));
        assert_eq!(
            notes.iter().map(|n| n.pitch).collect::<Vec<_>>(),
            vec![60, 64]
        );
        let hop = FFT_HOP as f64 / f64::from(SAMPLE_RATE);
        assert!((notes[0].start - 25.0 * hop).abs() < 1e-9);
        assert!((notes[0].end - 75.0 * hop).abs() < 1e-9);
        assert_eq!(notes[0].velocity, (127.0f32 * 0.8).round() as u8);
    }

    #[test]
    fn short_blips_are_dropped() {
        let (frame, onset) = activations(1, &[(39, 40, 48)]);
        assert!(notes(&frame, &onset, 1, HOP).is_empty());
    }
}
