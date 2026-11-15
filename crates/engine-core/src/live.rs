//! Live karaoke pitch (AGENTS.md §6.3): microphone samples in, pitch frames on the song timeline
//! out, each compared with the reference by §5's per-frame rule ([`score::compare_frame`]).
//!
//! [`LivePitch`] is [`pitch::track`] run incrementally: frame `i` is centred on capture sample
//! `i * hop` with the same window, so it produces exactly the frames the offline tracker would
//! for the same signal. It never runs on an audio thread: hosts feed it from their capture
//! drain (desktop writer thread data, web main thread).

use crate::input::StreamState;
use crate::pitch::{self, PitchFrame, PitchTrack, Yin};
use crate::score::{self, FrameComparison, FrameTotals, ScoreSettings};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Incremental YIN tracker with [`pitch::track`]'s window, hop and voicing rules.
pub struct LivePitch {
    yin: Yin,
    sample_rate: u32,
    window: usize,
    hop: usize,
    /// Samples from absolute capture index `base` on.
    buffer: Vec<f32>,
    base: usize,
    /// Next frame to emit (centred on capture sample `next * hop`).
    next: usize,
    frame: Vec<f32>,
}

impl LivePitch {
    pub fn new(sample_rate: u32) -> Self {
        let (window, hop) = pitch::track_geometry(sample_rate);
        Self {
            yin: Yin::new(window),
            sample_rate,
            window,
            hop,
            buffer: Vec::new(),
            base: 0,
            next: 0,
            frame: vec![0.0; window],
        }
    }

    pub fn hop_seconds(&self) -> f64 {
        self.hop as f64 / self.sample_rate as f64
    }

    /// Appends capture samples; returns `(frame index, pitch)` for every frame now complete.
    pub fn push(&mut self, samples: &[f32]) -> Vec<(usize, PitchFrame)> {
        self.buffer.extend_from_slice(samples);
        let half = self.window / 2;
        let available = self.base + self.buffer.len();
        let mut out = Vec::new();
        while self.next * self.hop + (self.window - half) <= available {
            let start = (self.next * self.hop) as isize - half as isize;
            for (k, v) in self.frame.iter_mut().enumerate() {
                let idx = start + k as isize;
                *v = if idx >= 0 {
                    self.buffer[idx as usize - self.base]
                } else {
                    0.0 // before the first sample: zero-padded, as in `pitch::track`
                };
            }
            out.push((
                self.next,
                pitch::frame_pitch(&mut self.yin, &self.frame, self.sample_rate),
            ));
            self.next += 1;
        }
        // Keep only what the next frame still needs.
        let keep_from = (self.next * self.hop).saturating_sub(half).max(self.base);
        self.buffer.drain(..keep_from - self.base);
        self.base = keep_from;
        out
    }
}

/// One live frame on the song timeline.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LiveFrame {
    /// Song time (seconds), latency removed.
    pub seconds: f64,
    pub pitch: PitchFrame,
    /// Against the reference frame at `seconds` (§5); `Silent` outside the song.
    pub comparison: FrameComparison,
}

/// Frames since the previous update, plus the running totals of the whole run.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LiveUpdate {
    pub frames: Vec<LiveFrame>,
    pub totals: FrameTotals,
    /// The microphone stream; a host sets `Failed` when it stopped delivering mid-run.
    pub input: StreamState,
}

/// A live run: capture sample `i` is song time `i / sample_rate − song_zero`.
pub struct LiveSession {
    pitch: LivePitch,
    reference: PitchTrack,
    settings: ScoreSettings,
    song_zero: f64,
    song_seconds: f64,
    totals: FrameTotals,
}

impl LiveSession {
    /// `song_zero`: capture time (seconds) of song time 0, with the round-trip latency included,
    /// exactly as recorded takes are placed (`take::timeline_from_capture`).
    pub fn new(
        reference: &PitchTrack,
        song_seconds: f64,
        settings: ScoreSettings,
        sample_rate: u32,
        song_zero: f64,
    ) -> Self {
        Self {
            pitch: LivePitch::new(sample_rate),
            reference: reference.clone(),
            settings,
            song_zero,
            song_seconds,
            totals: FrameTotals::default(),
        }
    }

    /// Feeds capture samples (in order, from capture sample 0).
    pub fn push(&mut self, samples: &[f32]) -> LiveUpdate {
        let hop = self.pitch.hop_seconds();
        let frames: Vec<LiveFrame> = self
            .pitch
            .push(samples)
            .into_iter()
            .filter_map(|(i, pitch)| {
                let seconds = i as f64 * hop - self.song_zero;
                // Count-in and lead-in frames are not part of the song.
                (seconds >= 0.0 && seconds <= self.song_seconds).then_some((seconds, pitch))
            })
            .map(|(seconds, pitch)| {
                let reference = self
                    .reference
                    .frame_at(seconds)
                    .copied()
                    .unwrap_or(PitchFrame::Unvoiced);
                let comparison = score::compare_frame(reference, pitch, self.settings);
                self.totals.add(comparison);
                LiveFrame {
                    seconds,
                    pitch,
                    comparison,
                }
            })
            .collect();
        LiveUpdate {
            frames,
            totals: self.totals,
            input: StreamState::Running,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::melody;
    use crate::note::midi_to_hz;
    use crate::pitch::tests::harmonic;
    use crate::score::{score, Difficulty, OctaveMode};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    const SR: u32 = 16_000;

    /// Four sung notes with gaps.
    fn singing(detune_cents: f32) -> Vec<f32> {
        let mut out = vec![0.0; SR as usize / 4];
        for midi in [57.0f32, 59.0, 60.0, 62.0] {
            out.extend(harmonic(midi_to_hz(midi + detune_cents / 100.0), 0.4, SR));
            out.extend(vec![0.0; SR as usize / 10]);
        }
        out
    }

    /// Feeds `signal` in uneven chunks.
    fn feed(session: &mut LiveSession, signal: &[f32]) -> Vec<LiveUpdate> {
        let sizes = [17usize, 128, 1000, 3, 441, 2048];
        let (mut at, mut k, mut out) = (0, 0, Vec::new());
        while at < signal.len() {
            let n = sizes[k % sizes.len()].min(signal.len() - at);
            out.push(session.push(&signal[at..at + n]));
            at += n;
            k += 1;
        }
        out
    }

    #[test]
    fn live_frames_match_the_offline_tracker() {
        let signal = singing(0.0);
        let offline = pitch::track(&signal, SR);
        let mut live = LivePitch::new(SR);
        let mut frames = Vec::new();
        for chunk in signal.chunks(300) {
            frames.extend(live.push(chunk));
        }
        assert!(!frames.is_empty());
        for (i, f) in &frames {
            assert_eq!(*f, offline.frames[*i], "frame {i}");
        }
        // Every frame whose window fits in the signal was emitted.
        assert!(frames.len() + 3 >= offline.frames.len());
    }

    #[test]
    fn live_scoring_matches_the_offline_scorer() {
        let reference_signal = singing(0.0);
        let reference = pitch::track(&reference_signal, SR);
        let notes = melody::segment(
            &reference,
            crate::key::Key {
                tonic: 0,
                mode: crate::key::Mode::Major,
            },
        );
        let song_seconds = reference_signal.len() as f64 / SR as f64;
        for (detune, difficulty) in [
            (0.0, Difficulty::Hard),
            (30.0, Difficulty::Hard),
            (30.0, Difficulty::Easy),
            (80.0, Difficulty::Easy),
        ] {
            let settings = ScoreSettings {
                difficulty,
                octave: OctaveMode::Fold,
            };
            let take_signal = singing(detune);
            let take = pitch::track(&take_signal, SR);
            let offline = score(&reference, &notes, &take, 0.0, settings);

            let mut session = LiveSession::new(&reference, song_seconds, settings, SR, 0.0);
            let updates = feed(&mut session, &take_signal);
            let live: Vec<LiveFrame> = updates.iter().flat_map(|u| u.frames.clone()).collect();
            for f in &live {
                let i = (f.seconds / reference.hop_seconds).round() as usize;
                assert_eq!(
                    f.comparison, offline.frames[i],
                    "detune {detune}, frame {i}"
                );
            }
            let totals = updates.last().unwrap().totals;
            assert_eq!(
                totals,
                FrameTotals::of(&live.iter().map(|f| f.comparison).collect::<Vec<_>>())
            );
            assert!(
                (totals.pitch() - FrameTotals::of(&offline.frames[..live.len()]).pitch()).abs()
                    < 1e-3
            );
        }
    }

    #[test]
    fn song_zero_places_frames_on_the_song_timeline() {
        let reference = pitch::track(&singing(0.0), SR);
        let settings = ScoreSettings::default();
        // One second of count-in and latency before the song starts.
        let mut capture = vec![0.0f32; SR as usize];
        capture.extend(singing(0.0));
        let mut session = LiveSession::new(&reference, 10.0, settings, SR, 1.0);
        let frames: Vec<LiveFrame> = feed(&mut session, &capture)
            .into_iter()
            .flat_map(|u| u.frames)
            .collect();
        assert!(frames.iter().all(|f| f.seconds >= 0.0));
        assert!(frames[0].seconds < 0.011);
        let hits = frames
            .iter()
            .filter(|f| matches!(f.comparison, FrameComparison::Hit { .. }))
            .count();
        assert!(hits > 100, "hits {hits}");
    }
}
