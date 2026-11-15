//! Export (AGENTS.md §8.8): the mixdown of the chosen player tracks at their mix levels, over the
//! song or the loop, mixed by the player's own law ([`mix_block`], so what you export is what
//! you hear) and written as 24-bit WAV. MP3 and M4A are encoded from that WAV by the host.

use crate::audio::StereoAudio;
use crate::player::{mix_block, MixParams, TrackMode};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ExportFormat {
    Wav24,
    Mp3,
    M4a,
}

impl ExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Wav24 => "wav",
            Self::Mp3 => "mp3",
            Self::M4a => "m4a",
        }
    }
}

/// Song time to export, in seconds.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum ExportRange {
    Song,
    Loop { start: f64, end: f64 },
}

/// What to export: player track indices (song tracks, then takes; see
/// [`crate::take::player_index`]), the format and the range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ExportRequest {
    pub tracks: Vec<u32>,
    pub format: ExportFormat,
    pub range: ExportRange,
}

/// Frames rendered per progress report.
pub const BLOCK: usize = 1 << 15;

/// One player track's audio and mix, borrowed (export never copies the song).
#[derive(Clone, Copy)]
pub struct TrackRef<'a> {
    pub left: &'a [f32],
    pub right: &'a [f32],
    pub mix: MixParams,
}

/// `[start, end)` frames of `range` in a song `length` frames long.
pub fn range_frames(range: ExportRange, sample_rate: u32, length: usize) -> (usize, usize) {
    match range {
        ExportRange::Song => (0, length),
        ExportRange::Loop { start, end } => {
            let at = |s: f64| ((s.max(0.0) * f64::from(sample_rate)) as usize).min(length);
            (at(start), at(end).max(at(start)))
        }
    }
}

/// Mixes the tracks listed in `include` (by index; solo is ignored: the checklist is the
/// selection) from song frame `from` into `left`/`right`, by the player's mixing law, clipped as
/// the player clips.
pub fn mix_chunk(
    tracks: &[TrackRef],
    include: &[u32],
    from: usize,
    left: &mut [f32],
    right: &mut [f32],
) {
    left.fill(0.0);
    right.fill(0.0);
    let chosen = tracks.iter().enumerate().map(|(i, t)| {
        let mode = if include.contains(&(i as u32)) {
            TrackMode::Normal
        } else {
            TrackMode::Muted
        };
        (t.left, t.right, MixParams { mode, ..t.mix })
    });
    mix_block(chosen, from, left, right);
    for v in left.iter_mut().chain(right.iter_mut()) {
        *v = v.clamp(-1.0, 1.0);
    }
}

/// The whole mixdown of `range`; `progress` sees 0–1.
pub fn mixdown(
    tracks: &[TrackRef],
    include: &[u32],
    sample_rate: u32,
    range: ExportRange,
    mut progress: impl FnMut(f32),
) -> StereoAudio {
    let length = tracks.iter().map(|t| t.left.len()).max().unwrap_or(0);
    let (start, end) = range_frames(range, sample_rate, length);
    let n = end - start;
    let mut left = vec![0.0; n];
    let mut right = vec![0.0; n];
    let mut done = 0;
    while done < n {
        let k = BLOCK.min(n - done);
        mix_chunk(
            tracks,
            include,
            start + done,
            &mut left[done..done + k],
            &mut right[done..done + k],
        );
        done += k;
        progress(done as f32 / n as f32);
    }
    StereoAudio {
        sample_rate,
        left,
        right,
    }
}

/// A 24-bit stereo PCM WAV file of `audio`.
pub fn encode_wav24(audio: &StereoAudio) -> Vec<u8> {
    let frames = audio.left.len();
    let data = (frames * 6) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend(b"RIFF");
    out.extend((36 + data).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes()); // PCM
    out.extend(2u16.to_le_bytes());
    out.extend(audio.sample_rate.to_le_bytes());
    out.extend((audio.sample_rate * 6).to_le_bytes());
    out.extend(6u16.to_le_bytes());
    out.extend(24u16.to_le_bytes());
    out.extend(b"data");
    out.extend(data.to_le_bytes());
    const FULL: f32 = 8_388_607.0;
    for (l, r) in audio.left.iter().zip(&audio.right) {
        for v in [l, r] {
            let s = (v.clamp(-1.0, 1.0) * FULL).round() as i32;
            out.extend(&s.to_le_bytes()[..3]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    const SR: u32 = 8_000;

    fn audio() -> Vec<Vec<f32>> {
        vec![
            (0..10_000).map(|i| 0.2 * (i as f32 * 0.01).sin()).collect(),
            (0..9_000).map(|i| 0.1 * (i as f32 * 0.03).cos()).collect(),
            vec![0.4; 10_000],
        ]
    }

    fn refs(a: &[Vec<f32>]) -> Vec<TrackRef<'_>> {
        let mix = |gain, pan| MixParams {
            gain,
            pan,
            mode: TrackMode::Muted,
        };
        vec![
            TrackRef {
                left: &a[0],
                right: &a[0],
                mix: mix(0.5, 0.0),
            },
            TrackRef {
                left: &a[1],
                right: &a[1],
                mix: mix(1.0, -0.5),
            },
            TrackRef {
                left: &a[2],
                right: &a[2],
                mix: mix(1.0, 0.0),
            },
        ]
    }

    #[test]
    fn mixdown_is_the_gain_and_pan_weighted_sum_of_the_chosen_tracks() {
        let a = audio();
        let t = refs(&a);
        let out = mixdown(&t, &[0, 1], SR, ExportRange::Song, |_| {});
        assert_eq!(out.left.len(), 10_000);
        for i in [0, 123, 4_567, 8_999, 9_500] {
            let a = t[0].left[i] * 0.5;
            let b = t[1].left.get(i).copied().unwrap_or(0.0);
            // Pan −0.5: left at full gain, right at half (the player's linear pan law).
            assert!((out.left[i] - (a + b)).abs() < 1e-6, "left {i}");
            assert!((out.right[i] - (a + 0.5 * b)).abs() < 1e-6, "right {i}");
        }
    }

    #[test]
    fn a_loop_range_exports_only_the_loop() {
        let a = audio();
        let t = refs(&a);
        let whole = mixdown(&t, &[2], SR, ExportRange::Song, |_| {});
        let part = mixdown(
            &t,
            &[2],
            SR,
            ExportRange::Loop {
                start: 0.25,
                end: 0.5,
            },
            |_| {},
        );
        assert_eq!(part.left.len(), 2_000);
        assert_eq!(part.left[..], whole.left[2_000..4_000]);
    }

    #[test]
    fn progress_reaches_one() {
        let mut last = 0.0;
        mixdown(&refs(&audio()), &[0], SR, ExportRange::Song, |p| last = p);
        assert_eq!(last, 1.0);
    }

    #[test]
    fn wav24_decodes_back_to_the_mixdown() {
        let out = mixdown(&refs(&audio()), &[0, 1, 2], SR, ExportRange::Song, |_| {});
        let bytes = encode_wav24(&out);
        assert_eq!(bytes.len(), 44 + out.left.len() * 6);
        let (mono, rate) = crate::wav::decode_mono(&bytes).unwrap();
        assert_eq!(rate, SR);
        for i in (0..mono.len()).step_by(997) {
            let want = 0.5 * (out.left[i] + out.right[i]);
            assert!((mono[i] - want).abs() < 1e-5, "{i}: {} vs {want}", mono[i]);
        }
    }
}
