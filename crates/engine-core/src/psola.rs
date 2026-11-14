//! Live auto-tune (AGENTS.md §7 step 3): microphone in, corrected voice out, on the audio thread.
//!
//! [`LiveTune`] runs the shared analysis ([`pitch::frame_pitch`], the offline tracker's window
//! and hop) and [`Tuner`] every hop, and feeds [`Psola`], a time-domain pitch-synchronous
//! overlap-add shifter. TD-PSOLA moves pitch by re-spacing two-period grains cut at the
//! voice's pitch marks, so the spectral envelope inside each grain (the formants) is kept as
//! sung; a formant shift resamples the grains themselves.
//!
//! Everything is allocated in `new`; `process` never allocates, locks or blocks (§9). The
//! output is delayed by a fixed [`Psola::latency`]: a grain can only be cut once a period and a
//! half of voice past its mark has arrived.

use crate::note::midi_to_hz;
use crate::pitch::{self, PitchFrame, Yin};
use crate::tune::{Correction, TuneSettings, Tuner, MAX_FORMANT_SEMITONES};
use std::collections::VecDeque;
use std::f32::consts::PI;

/// Lowest voice the live shifter tracks; it sets the grain size and so the latency.
pub const LIVE_MIN_HZ: f32 = 90.0;
/// Highest voice (periods shorter than this are clamped).
const LIVE_MAX_HZ: f32 = 1100.0;
/// Mark spacing while unvoiced: any steady spacing reconstructs the input (ratio 1).
const UNVOICED_HZ: f32 = 200.0;
/// Ring capacity (samples, power of two): latency + a grain + the largest host block.
const RING: usize = 1 << 14;
/// Analysis marks kept (a few periods of history and lookahead).
const MARKS: usize = 64;

/// A pitch mark: where a period's grain is centred, and its period (samples).
#[derive(Debug, Clone, Copy)]
struct Mark {
    at: u64,
    period: u32,
}

/// Streaming TD-PSOLA pitch shifter. Mono, any block size up to [`RING`]/4.
pub struct Psola {
    max_period: usize,
    min_period: usize,
    unvoiced_period: usize,
    latency: usize,
    /// Input history and the output being overlap-added, both indexed by absolute sample.
    input: Vec<f32>,
    output: Vec<f32>,
    written: u64,
    marks: VecDeque<Mark>,
    /// Next synthesis mark (absolute sample, fractional).
    next_synthesis: f64,
    /// Control, set by [`Psola::set`].
    period: Option<f32>,
    ratio: f32,
    formant: f32,
}

impl Psola {
    pub fn new(sample_rate: u32) -> Self {
        let max_period = (sample_rate as f32 / LIVE_MIN_HZ).ceil() as usize;
        let max_formant = 2f32.powf(MAX_FORMANT_SEMITONES / 12.0);
        let mut marks = VecDeque::with_capacity(MARKS);
        let unvoiced_period = (sample_rate as f32 / UNVOICED_HZ) as usize;
        marks.push_back(Mark {
            at: 0,
            period: unvoiced_period as u32,
        });
        Self {
            max_period,
            min_period: (sample_rate as f32 / LIVE_MAX_HZ) as usize,
            unvoiced_period,
            // A grain reaches a period past its mark (scaled by the formant shift) and the
            // newest mark can trail the input by a quarter period more than its period; a grain
            // starts half a period ahead of the sample being played.
            latency: ((max_formant + 1.5) * max_period as f32).ceil() as usize,
            input: vec![0.0; RING],
            output: vec![0.0; RING],
            written: 0,
            marks,
            next_synthesis: 0.0,
            period: None,
            ratio: 1.0,
            formant: 1.0,
        }
    }

    /// Output delay in samples.
    pub fn latency(&self) -> usize {
        self.latency
    }

    /// The voice's current period (`None` when unvoiced), the pitch ratio to apply, and the
    /// formant ratio.
    pub fn set(&mut self, period: Option<f32>, ratio: f32, formant: f32) {
        self.period = period.map(|p| p.clamp(self.min_period as f32, self.max_period as f32));
        self.ratio = ratio.clamp(0.25, 4.0);
        self.formant = formant;
    }

    /// The input sample `at` (0 outside what has arrived), linearly interpolated.
    fn input_at(&self, at: f64) -> f32 {
        if at < 0.0 || at + 1.0 >= self.written as f64 {
            return 0.0;
        }
        let i = at.floor() as u64;
        let t = (at - i as f64) as f32;
        let a = self.input[i as usize % RING];
        let b = self.input[(i + 1) as usize % RING];
        a + t * (b - a)
    }

    /// Places the analysis marks the input now allows: one period after the last, at the
    /// largest sample within a quarter period of it while voiced (peak picking), exactly a
    /// period on while unvoiced.
    fn find_marks(&mut self) {
        loop {
            let last = *self.marks.back().expect("marks are never empty");
            let mark = match self.period {
                Some(p) => {
                    let p = p as u64;
                    let (lo, hi) = (last.at + p - p / 4, last.at + p + p / 4);
                    if hi >= self.written {
                        return;
                    }
                    let at = (lo..=hi)
                        .max_by(|a, b| {
                            self.input[*a as usize % RING]
                                .total_cmp(&self.input[*b as usize % RING])
                        })
                        .expect("non-empty range");
                    Mark {
                        at,
                        period: (at - last.at) as u32,
                    }
                }
                None => {
                    let at = last.at + self.unvoiced_period as u64;
                    if at >= self.written {
                        return;
                    }
                    Mark {
                        at,
                        period: self.unvoiced_period as u32,
                    }
                }
            };
            if self.marks.len() == MARKS {
                self.marks.pop_front();
            }
            self.marks.push_back(mark);
        }
    }

    /// Overlap-adds grains at synthesis marks up to `until`: each takes the analysis mark
    /// nearest to it whose grain has fully arrived, and the next comes the mark's period
    /// divided by the ratio later. It never waits, so playback never misses a grain.
    fn synthesize(&mut self, until: f64) {
        while self.next_synthesis <= until {
            let t = self.next_synthesis;
            while self.marks.len() > 1 && (self.marks[1].at as f64) <= t {
                self.marks.pop_front();
            }
            let usable = |m: &Mark| {
                let half = m.period.min(self.max_period as u32) as f64;
                m.at as f64 + self.formant as f64 * half + 1.0 < self.written as f64
            };
            let index = match (self.marks.front(), self.marks.get(1)) {
                (Some(a), Some(b)) if usable(b) && (b.at as f64 - t) < (t - a.at as f64) => 1,
                (Some(a), _) if usable(a) => 0,
                _ => {
                    // Before the first grain has arrived: nothing to play yet.
                    self.next_synthesis += self.unvoiced_period as f64;
                    continue;
                }
            };
            let mark = self.marks[index];
            // The grain rises over the period before its mark and falls over the one after
            // (the last period again while the next mark is unknown). At ratio 1 synthesis
            // marks then land on the analysis marks and neighbouring grains sum to exactly 1.
            let next = self.marks.get(index + 1).map_or(mark.period, |n| n.period);
            let clamp = |p: u32| p.clamp(self.min_period as u32, self.max_period as u32) as i64;
            let (left, right) = (clamp(mark.period), clamp(next));
            // More grains overlap when shifting up: keep the level.
            let gain = 1.0 / self.ratio.max(1.0);
            let centre = t.round() as i64;
            for i in -left..=right {
                let half = if i < 0 { left } else { right };
                let w = 0.5 * (1.0 + (PI * i as f32 / half as f32).cos());
                let source = self.input_at(mark.at as f64 + i as f64 * self.formant as f64);
                let at = (centre + i).max(0) as usize % RING;
                self.output[at] += gain * w * source;
            }
            self.next_synthesis += f64::from(next) / f64::from(self.ratio);
        }
    }

    /// Shifts `input` into `output` (same length), [`Psola::latency`] samples late. `dry`
    /// receives the input with the same delay, for a dry/wet mix.
    pub fn process(&mut self, input: &[f32], output: &mut [f32], dry: &mut [f32]) {
        for (k, &x) in input.iter().enumerate() {
            self.input[(self.written as usize + k) % RING] = x;
        }
        let start = self.written;
        self.written += input.len() as u64;
        self.find_marks();
        let end = self.written as i64 - self.latency as i64;
        self.synthesize((end + self.max_period as i64) as f64);
        for k in 0..input.len() {
            let n = start as i64 + k as i64 - self.latency as i64;
            if n < 0 {
                output[k] = 0.0;
                dry[k] = 0.0;
                continue;
            }
            let slot = n as usize % RING;
            output[k] = self.output[slot];
            self.output[slot] = 0.0;
            dry[k] = self.input[slot];
        }
    }
}

/// The live monitor's voice chain: pitch analysis, [`Tuner`], [`Psola`] and the dry/wet mix.
pub struct LiveTune {
    sample_rate: u32,
    yin: Yin,
    tuner: Tuner,
    psola: Psola,
    /// Analysis window (the offline tracker's) and hop.
    frame: Vec<f32>,
    hop: usize,
    /// Samples since the last analysis.
    since: usize,
    history: Vec<f32>,
    written: usize,
    wet: Vec<f32>,
    dry: Vec<f32>,
    last: Correction,
}

impl LiveTune {
    /// `max_block`: the largest block `process` will be given.
    pub fn new(sample_rate: u32, settings: TuneSettings, max_block: usize) -> Self {
        let (window, hop) = pitch::track_geometry(sample_rate);
        Self {
            sample_rate,
            yin: Yin::new(window),
            tuner: Tuner::new(settings, hop as f32 / sample_rate as f32),
            psola: Psola::new(sample_rate),
            frame: vec![0.0; window],
            hop,
            since: 0,
            history: vec![0.0; window],
            written: 0,
            wet: vec![0.0; max_block],
            dry: vec![0.0; max_block],
            last: Correction {
                target: None,
                semitones: 0.0,
            },
        }
    }

    pub fn set_settings(&mut self, settings: TuneSettings) {
        self.tuner.set_settings(settings);
    }

    /// Output delay in samples.
    pub fn latency(&self) -> usize {
        self.psola.latency()
    }

    /// The latest correction (for meters; `target: None` while unvoiced).
    pub fn correction(&self) -> Correction {
        self.last
    }

    /// Analyses the latest window and updates the shifter.
    fn analyse(&mut self, melody: Option<f32>) {
        let w = self.history.len();
        for (k, v) in self.frame.iter_mut().enumerate() {
            *v = self.history[(self.written + k) % w];
        }
        let frame = pitch::frame_pitch(&mut self.yin, &self.frame, self.sample_rate);
        self.last = self.tuner.next(frame, melody);
        let settings = self.tuner.settings();
        let period = match frame {
            PitchFrame::Voiced { midi } => Some(self.sample_rate as f32 / midi_to_hz(midi)),
            PitchFrame::Unvoiced => None,
        };
        let ratio = 2f32.powf(self.last.semitones / 12.0);
        self.psola.set(period, ratio, settings.formant_factor());
    }

    /// Tunes `input` into `output` (same length, at most `max_block`), mixed dry/wet per the
    /// settings. `melody`: the reference note now (follow-melody mode).
    pub fn process(&mut self, input: &[f32], output: &mut [f32], melody: Option<f32>) {
        let mut done = 0;
        while done < input.len() {
            let n = (self.hop - self.since).min(input.len() - done);
            let chunk = &input[done..done + n];
            let w = self.history.len();
            for &x in chunk {
                self.history[self.written % w] = x;
                self.written += 1;
            }
            self.psola.process(
                chunk,
                &mut self.wet[done..done + n],
                &mut self.dry[done..done + n],
            );
            self.since += n;
            if self.since == self.hop {
                self.since = 0;
                self.analyse(melody);
            }
            done += n;
        }
        let mix = self.tuner.settings().mix;
        for ((o, w), d) in output.iter_mut().zip(&self.wet).zip(&self.dry) {
            *o = mix * w + (1.0 - mix) * d;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::note::cents;
    use crate::shift::tests::{formant_error, median_hz, resampled, vowel};
    use crate::tune::{Scale, TuneMode};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    const SR: u32 = 44_100;
    const BLOCK: usize = 128;

    /// `input` through a fixed-ratio [`Psola`] in 128-sample blocks, latency removed.
    fn fixed(input: &[f32], hz: f32, semitones: f32, formant: f32) -> Vec<f32> {
        let mut psola = Psola::new(SR);
        psola.set(Some(SR as f32 / hz), 2f32.powf(semitones / 12.0), formant);
        let mut out = vec![0.0; input.len() + psola.latency() + BLOCK];
        let mut dry = vec![0.0; BLOCK];
        let padded: Vec<f32> = input
            .iter()
            .copied()
            .chain(std::iter::repeat(0.0))
            .take(out.len())
            .collect();
        for (i, o) in padded.chunks(BLOCK).zip(out.chunks_mut(BLOCK)) {
            psola.process(i, o, &mut dry[..i.len()]);
        }
        out.drain(..psola.latency());
        out.truncate(input.len());
        out
    }

    #[test]
    fn shifts_pitch_within_five_cents() {
        let input = vowel(220.0, 1.0, SR);
        for semitones in [1.0f32, -1.0, 3.0, -3.0, 12.0, -12.0] {
            let got = median_hz(&fixed(&input, 220.0, semitones, 1.0), SR);
            let want = 220.0 * 2f32.powf(semitones / 12.0);
            assert!(
                cents(got, want).abs() < 5.0,
                "{semitones:+} st: {got} Hz vs {want} Hz"
            );
        }
    }

    #[test]
    fn keeps_the_formants() {
        let input = vowel(196.0, 1.0, SR);
        let ratio = 2f32.powf(3.0 / 12.0);
        let ours = formant_error(&fixed(&input, 196.0, 3.0, 1.0), 196.0 * ratio, SR);
        let chipmunk = formant_error(&resampled(&input, ratio), 196.0 * ratio, SR);
        assert!(
            ours < chipmunk / 2.0,
            "PSOLA {ours} dB vs resampling {chipmunk} dB"
        );
    }

    #[test]
    fn unity_ratio_reconstructs_the_input() {
        let input = vowel(220.0, 0.5, SR);
        let out = fixed(&input, 220.0, 0.0, 1.0);
        let error: f32 = input.iter().zip(&out).map(|(a, b)| (a - b).powi(2)).sum();
        let energy: f32 = input.iter().map(|a| a * a).sum();
        let snr = 10.0 * (energy / error).log10();
        assert!(snr > 20.0, "SNR {snr} dB");
    }

    fn hard_chromatic() -> TuneSettings {
        TuneSettings {
            mode: TuneMode::Scale,
            scale: Scale::Chromatic,
            retune_ms: 0.0,
            humanize: 0.0,
            formant_semitones: 0.0,
            mix: 1.0,
        }
    }

    #[test]
    fn live_tune_pulls_a_sharp_note_onto_pitch() {
        // A3 sung 35 cents sharp.
        let sung = 220.0 * 2f32.powf(0.35 / 12.0);
        let input = vowel(sung, 1.5, SR);
        let mut tune = LiveTune::new(SR, hard_chromatic(), BLOCK);
        let mut out = vec![0.0; input.len()];
        for (i, o) in input.chunks(BLOCK).zip(out.chunks_mut(BLOCK)) {
            tune.process(i, o, None);
        }
        assert_eq!(tune.correction().target, Some(57.0));
        let got = median_hz(&out[out.len() / 3..], SR);
        assert!(cents(got, 220.0).abs() < 5.0, "{got} Hz");
        // Latency: under 35 ms at 44.1 kHz.
        let ms = tune.latency() as f32 * 1000.0 / SR as f32;
        assert!(ms < 35.0, "{ms} ms");
    }

    #[test]
    fn dry_mix_is_the_delayed_input() {
        let input = vowel(250.0, 0.5, SR);
        let mut tune = LiveTune::new(
            SR,
            TuneSettings {
                mix: 0.0,
                ..hard_chromatic()
            },
            BLOCK,
        );
        let mut out = vec![0.0; input.len()];
        for (i, o) in input.chunks(BLOCK).zip(out.chunks_mut(BLOCK)) {
            tune.process(i, o, None);
        }
        let d = tune.latency();
        assert!(out[d..]
            .iter()
            .zip(&input)
            .all(|(o, i)| (o - i).abs() < 1e-6));
    }

    /// The live chain must use under 25% of one core at 128-frame blocks (AGENTS.md §7 step 3).
    /// Timed in release builds only (`cargo test --release -p engine-core live_cpu`).
    #[test]
    #[cfg_attr(any(debug_assertions, target_arch = "wasm32"), ignore)]
    fn live_cpu_under_a_quarter_core() {
        let seconds = 10.0;
        let input = vowel(180.0, seconds, SR);
        let mut tune = LiveTune::new(
            SR,
            TuneSettings::for_key(crate::key::Key {
                tonic: 0,
                mode: crate::key::Mode::Major,
            }),
            BLOCK,
        );
        let mut out = vec![0.0; BLOCK];
        let started = std::time::Instant::now();
        for i in input.chunks(BLOCK) {
            tune.process(i, &mut out[..i.len()], None);
        }
        let load = started.elapsed().as_secs_f32() / seconds;
        eprintln!(
            "live auto-tune: {:.2}% of one core at {BLOCK}-frame blocks, latency {:.1} ms",
            load * 100.0,
            tune.latency() as f32 * 1000.0 / SR as f32
        );
        assert!(load < 0.25, "{load}");
    }
}
