//! The vocal chain (AGENTS.md §8, Produce → Vocal chain), pure DSP: high-pass → de-esser →
//! compressor → EQ (low shelf, presence, air shelf) → saturation → reverb and delay sends.
//!
//! [`VocalChain`] processes blocks in place and never allocates after [`VocalChain::new`] (its
//! reverb and delay lines are sized there), so the live monitor runs it on the audio thread
//! (§9); [`render`] runs it over a take offline. [`measure`] reads a take's loudness (BS.1770
//! integrated LUFS), spectral tilt and sibilance; [`settings`] turns a [`ChainPreset`] and those
//! measures into [`ChainSettings`]: every preset sets its thresholds and output gain from the
//! take's level, and `Auto` also corrects the take's tone (bright or dark, sibilant or not).

use crate::stft::Stft;
use serde::{Deserialize, Serialize};
use std::f32::consts::TAU;
use ts_rs::TS;

/// Every stage's parameters. A stage at its neutral value (0 dB, ratio 1, mix 0) is bypassed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChainSettings {
    /// High-pass corner; 0 = off.
    pub highpass_hz: f32,
    /// Where the de-esser's sidechain band starts (it ducks the whole voice while the band is
    /// loud: no band-split phase smear, and sibilants rarely overlap vowels).
    pub deess_hz: f32,
    /// Band level (dBFS) above which sibilance is pulled down.
    pub deess_threshold_db: f32,
    /// Most the band is pulled down; 0 = off.
    pub deess_max_db: f32,
    pub comp_threshold_db: f32,
    /// 1 = off.
    pub comp_ratio: f32,
    pub comp_makeup_db: f32,
    /// Low shelf at [`LOW_HZ`].
    pub low_db: f32,
    /// Peak at [`PRESENCE_HZ`].
    pub presence_db: f32,
    /// High shelf at [`AIR_HZ`].
    pub air_db: f32,
    /// Saturation, 0–1.
    pub drive: f32,
    /// Reverb send, 0–1.
    pub reverb_mix: f32,
    /// Reverb decay, 0–1.
    pub reverb_size: f32,
    /// Delay send, 0–1.
    pub delay_mix: f32,
    pub delay_ms: f32,
    pub output_db: f32,
}

impl ChainSettings {
    /// Every stage bypassed.
    pub const OFF: Self = Self {
        highpass_hz: 0.0,
        deess_hz: 6000.0,
        deess_threshold_db: 0.0,
        deess_max_db: 0.0,
        comp_threshold_db: 0.0,
        comp_ratio: 1.0,
        comp_makeup_db: 0.0,
        low_db: 0.0,
        presence_db: 0.0,
        air_db: 0.0,
        drive: 0.0,
        reverb_mix: 0.0,
        reverb_size: 0.5,
        delay_mix: 0.0,
        delay_ms: 375.0,
        output_db: 0.0,
    };

    /// Every value in its range.
    pub fn clamped(self) -> Self {
        Self {
            highpass_hz: self.highpass_hz.clamp(0.0, 400.0),
            deess_hz: self.deess_hz.clamp(3000.0, 12000.0),
            deess_threshold_db: self.deess_threshold_db.clamp(-80.0, 0.0),
            deess_max_db: self.deess_max_db.clamp(0.0, 18.0),
            comp_threshold_db: self.comp_threshold_db.clamp(-60.0, 0.0),
            comp_ratio: self.comp_ratio.clamp(1.0, 20.0),
            comp_makeup_db: self.comp_makeup_db.clamp(0.0, 24.0),
            low_db: self.low_db.clamp(-12.0, 12.0),
            presence_db: self.presence_db.clamp(-12.0, 12.0),
            air_db: self.air_db.clamp(-12.0, 12.0),
            drive: self.drive.clamp(0.0, 1.0),
            reverb_mix: self.reverb_mix.clamp(0.0, 1.0),
            reverb_size: self.reverb_size.clamp(0.0, 1.0),
            delay_mix: self.delay_mix.clamp(0.0, 1.0),
            delay_ms: self.delay_ms.clamp(20.0, MAX_DELAY_SECONDS * 1000.0),
            output_db: self.output_db.clamp(-24.0, 24.0),
        }
    }
}

/// Starting points for the chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum ChainPreset {
    /// Pop's dynamics with the take's tone corrected from its measures.
    Auto,
    /// Clean-up only: high-pass, gentle de-essing and compression.
    Natural,
    Pop,
    Rnb,
    Rock,
    HipHop,
    Ballad,
}

impl ChainPreset {
    pub const ALL: [Self; 7] = [
        Self::Auto,
        Self::Natural,
        Self::Pop,
        Self::Rnb,
        Self::Rock,
        Self::HipHop,
        Self::Ballad,
    ];
}

/// What [`measure`] reads from a take.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Measures {
    /// Integrated loudness (ITU-R BS.1770), LUFS; [`SILENCE_LUFS`] when silent.
    pub lufs: f32,
    /// Sample peak, dBFS.
    pub peak_db: f32,
    /// Energy of 2–6 kHz over 150 Hz–1 kHz, dB (higher is brighter).
    pub tilt_db: f32,
    /// Energy of 5–10 kHz over 150 Hz–5 kHz, dB (higher is more sibilant).
    pub sibilance_db: f32,
}

/// A chain's effect on a take: its measures before and after.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ChainReport {
    pub before: Measures,
    pub after: Measures,
}

pub const LOW_HZ: f32 = 200.0;
pub const PRESENCE_HZ: f32 = 3000.0;
pub const AIR_HZ: f32 = 10000.0;
pub const SILENCE_LUFS: f32 = -70.0;
/// Where `Auto` (and every preset's output gain) aims a take's loudness.
pub const TARGET_LUFS: f32 = -16.0;
/// A typical sung take's tilt and sibilance (see [`Measures`]); `Auto` corrects towards them.
const NEUTRAL_TILT_DB: f32 = -14.0;
const NEUTRAL_SIBILANCE_DB: f32 = -22.0;
const MAX_DELAY_SECONDS: f32 = 2.0;
const DELAY_FEEDBACK: f32 = 0.3;
const COMP_ATTACK_MS: f32 = 5.0;
const COMP_RELEASE_MS: f32 = 100.0;
const DEESS_ATTACK_MS: f32 = 1.0;
const DEESS_RELEASE_MS: f32 = 60.0;
const DEESS_RATIO: f32 = 4.0;

fn db_to_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

fn gain_to_db(g: f32) -> f32 {
    20.0 * g.max(1e-9).log10()
}

/// One-pole smoothing coefficient for a time constant.
fn smoothing(ms: f32, sample_rate: f32) -> f32 {
    (-1.0 / (ms * 0.001 * sample_rate)).exp()
}

/// RBJ cookbook filter shapes.
#[derive(Clone, Copy)]
enum Shape {
    Highpass { q: f32 },
    LowShelf { db: f32 },
    HighShelf { db: f32 },
    Peak { db: f32, q: f32 },
}

/// A biquad (transposed direct form II).
#[derive(Clone, Copy, Default)]
struct Biquad {
    b: [f32; 3],
    a: [f32; 2],
    z: [f32; 2],
}

impl Biquad {
    fn new(shape: Shape, hz: f32, sample_rate: f32) -> Self {
        let mut f = Self::default();
        f.set(shape, hz, sample_rate);
        f
    }

    /// New coefficients, keeping the state (no clicks on parameter changes).
    fn set(&mut self, shape: Shape, hz: f32, sample_rate: f32) {
        let w = TAU * hz.min(0.49 * sample_rate) / sample_rate;
        let (sin, cos) = w.sin_cos();
        let (b, a) = match shape {
            Shape::Highpass { q } => {
                let alpha = sin / (2.0 * q);
                (
                    [(1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0],
                    [1.0 + alpha, -2.0 * cos, 1.0 - alpha],
                )
            }
            Shape::Peak { db, q } => {
                let g = 10f32.powf(db / 40.0);
                let alpha = sin / (2.0 * q);
                (
                    [1.0 + alpha * g, -2.0 * cos, 1.0 - alpha * g],
                    [1.0 + alpha / g, -2.0 * cos, 1.0 - alpha / g],
                )
            }
            Shape::LowShelf { db } | Shape::HighShelf { db } => {
                let g = 10f32.powf(db / 40.0);
                // Shelf slope 1.
                let alpha = sin / 2.0 * 2f32.sqrt();
                let k = 2.0 * g.sqrt() * alpha;
                let low = matches!(shape, Shape::LowShelf { .. });
                let s = if low { 1.0 } else { -1.0 };
                (
                    [
                        g * ((g + 1.0) - s * (g - 1.0) * cos + k),
                        s * 2.0 * g * ((g - 1.0) - s * (g + 1.0) * cos),
                        g * ((g + 1.0) - s * (g - 1.0) * cos - k),
                    ],
                    [
                        (g + 1.0) + s * (g - 1.0) * cos + k,
                        -s * 2.0 * ((g - 1.0) + s * (g + 1.0) * cos),
                        (g + 1.0) + s * (g - 1.0) * cos - k,
                    ],
                )
            }
        };
        self.b = [b[0] / a[0], b[1] / a[0], b[2] / a[0]];
        self.a = [a[1] / a[0], a[2] / a[0]];
    }

    fn run(&mut self, x: f32) -> f32 {
        let y = self.b[0] * x + self.z[0];
        self.z[0] = self.b[1] * x - self.a[0] * y + self.z[1];
        self.z[1] = self.b[2] * x - self.a[1] * y;
        y
    }
}

/// A peak envelope follower (linear).
#[derive(Clone, Copy, Default)]
struct Envelope {
    attack: f32,
    release: f32,
    level: f32,
}

impl Envelope {
    fn new(attack_ms: f32, release_ms: f32, sample_rate: f32) -> Self {
        Self {
            attack: smoothing(attack_ms, sample_rate),
            release: smoothing(release_ms, sample_rate),
            level: 0.0,
        }
    }

    fn run(&mut self, x: f32) -> f32 {
        let x = x.abs();
        let c = if x > self.level {
            self.attack
        } else {
            self.release
        };
        self.level = x + c * (self.level - x);
        self.level
    }
}

/// Gain reduction (dB, ≥ 0) of a compressor at `level_db`.
fn reduction(level_db: f32, threshold_db: f32, ratio: f32) -> f32 {
    ((level_db - threshold_db) * (1.0 - 1.0 / ratio)).max(0.0)
}

/// A fixed-length delay line.
struct Line {
    buf: Vec<f32>,
    pos: usize,
}

impl Line {
    fn new(len: usize) -> Self {
        Self {
            buf: vec![0.0; len.max(1)],
            pos: 0,
        }
    }

    /// The sample written `len` samples ago (the whole line by default).
    fn read(&self, len: usize) -> f32 {
        let n = self.buf.len();
        self.buf[(self.pos + n - len.clamp(1, n)) % n]
    }

    fn write(&mut self, x: f32) {
        self.buf[self.pos] = x;
        self.pos = (self.pos + 1) % self.buf.len();
    }

    fn clear(&mut self) {
        self.buf.fill(0.0);
    }
}

/// Freeverb's tunings (at 44.1 kHz).
const COMBS: [usize; 4] = [1116, 1188, 1277, 1356];
const ALLPASSES: [usize; 2] = [556, 441];
const REVERB_DAMP: f32 = 0.3;

/// A small Schroeder–Moorer reverb (Freeverb's structure, mono, four combs).
struct Reverb {
    combs: [Line; 4],
    damp: [f32; 4],
    allpasses: [Line; 2],
    feedback: f32,
}

impl Reverb {
    fn new(sample_rate: f32) -> Self {
        let scale = |n: usize| (n as f32 * sample_rate / 44_100.0) as usize;
        Self {
            combs: COMBS.map(|n| Line::new(scale(n))),
            damp: [0.0; 4],
            allpasses: ALLPASSES.map(|n| Line::new(scale(n))),
            feedback: 0.84,
        }
    }

    fn set_size(&mut self, size: f32) {
        self.feedback = 0.7 + 0.28 * size;
    }

    fn run(&mut self, x: f32) -> f32 {
        let mut sum = 0.0;
        for (line, damp) in self.combs.iter_mut().zip(&mut self.damp) {
            let y = line.read(usize::MAX);
            *damp = y * (1.0 - REVERB_DAMP) + *damp * REVERB_DAMP;
            line.write(x + *damp * self.feedback);
            sum += y;
        }
        let mut y = sum * 0.25;
        for line in &mut self.allpasses {
            let b = line.read(usize::MAX);
            line.write(y + b * 0.5);
            y = b - y;
        }
        y
    }

    fn clear(&mut self) {
        self.combs.iter_mut().for_each(Line::clear);
        self.allpasses.iter_mut().for_each(Line::clear);
        self.damp = [0.0; 4];
    }
}

/// The chain, ready to process blocks.
pub struct VocalChain {
    sample_rate: f32,
    settings: ChainSettings,
    highpass: [Biquad; 2],
    deess_band: Biquad,
    deess_env: Envelope,
    comp_env: Envelope,
    eq: [Biquad; 3],
    reverb: Reverb,
    delay: Line,
}

impl VocalChain {
    /// Allocates every buffer the chain needs; nothing allocates afterwards.
    pub fn new(sample_rate: u32) -> Self {
        let sr = sample_rate as f32;
        let mut chain = Self {
            sample_rate: sr,
            settings: ChainSettings::OFF,
            highpass: [Biquad::default(); 2],
            deess_band: Biquad::default(),
            deess_env: Envelope::new(DEESS_ATTACK_MS, DEESS_RELEASE_MS, sr),
            comp_env: Envelope::new(COMP_ATTACK_MS, COMP_RELEASE_MS, sr),
            eq: [Biquad::default(); 3],
            reverb: Reverb::new(sr),
            delay: Line::new((MAX_DELAY_SECONDS * sr) as usize + 1),
        };
        chain.set(ChainSettings::OFF);
        chain
    }

    /// New settings from the next sample on (no allocation; filter states are kept).
    pub fn set(&mut self, settings: ChainSettings) {
        let s = settings.clamped();
        let sr = self.sample_rate;
        // Two cascaded 2nd-order sections: 24 dB/octave (Butterworth Q for each half).
        for (f, q) in self.highpass.iter_mut().zip([0.541, 1.307]) {
            f.set(Shape::Highpass { q }, s.highpass_hz.max(1.0), sr);
        }
        self.deess_band
            .set(Shape::Highpass { q: 0.707 }, s.deess_hz, sr);
        self.eq[0].set(Shape::LowShelf { db: s.low_db }, LOW_HZ, sr);
        self.eq[1].set(
            Shape::Peak {
                db: s.presence_db,
                q: 0.9,
            },
            PRESENCE_HZ,
            sr,
        );
        self.eq[2].set(Shape::HighShelf { db: s.air_db }, AIR_HZ, sr);
        self.reverb.set_size(s.reverb_size);
        self.settings = s;
    }

    pub fn settings(&self) -> ChainSettings {
        self.settings
    }

    /// Forgets the signal so far (filter states, envelopes, the reverb and delay tails).
    pub fn reset(&mut self) {
        for f in self.highpass.iter_mut().chain(self.eq.iter_mut()) {
            f.z = [0.0; 2];
        }
        self.deess_band.z = [0.0; 2];
        self.deess_env.level = 0.0;
        self.comp_env.level = 0.0;
        self.reverb.clear();
        self.delay.clear();
    }

    /// Processes `block` in place.
    pub fn process(&mut self, block: &mut [f32]) {
        let s = self.settings;
        let delay = (s.delay_ms * 0.001 * self.sample_rate) as usize;
        let output = db_to_gain(s.output_db);
        let makeup = s.comp_makeup_db;
        let drive = 1.0 + 3.0 * s.drive;
        for v in block.iter_mut() {
            let mut x = *v;
            if s.highpass_hz > 0.0 {
                x = self.highpass[0].run(x);
                x = self.highpass[1].run(x);
            }
            if s.deess_max_db > 0.0 {
                let band = self.deess_band.run(x);
                let level = gain_to_db(self.deess_env.run(band));
                let cut = reduction(level, s.deess_threshold_db, DEESS_RATIO).min(s.deess_max_db);
                x *= db_to_gain(-cut);
            }
            if s.comp_ratio > 1.0 || makeup > 0.0 {
                let level = gain_to_db(self.comp_env.run(x));
                let cut = reduction(level, s.comp_threshold_db, s.comp_ratio);
                x *= db_to_gain(makeup - cut);
            }
            for f in &mut self.eq {
                x = f.run(x);
            }
            if s.drive > 0.0 {
                x = (drive * x).tanh() / drive;
            }
            let mut y = x;
            if s.reverb_mix > 0.0 {
                y += s.reverb_mix * self.reverb.run(x);
            }
            if s.delay_mix > 0.0 {
                let echo = self.delay.read(delay);
                self.delay.write(x + DELAY_FEEDBACK * echo);
                y += s.delay_mix * echo;
            }
            *v = y * output;
        }
    }
}

/// `signal` through the chain (offline; same length).
pub fn render(signal: &[f32], sample_rate: u32, settings: ChainSettings) -> Vec<f32> {
    let mut chain = VocalChain::new(sample_rate);
    chain.set(settings);
    let mut out = signal.to_vec();
    chain.process(&mut out);
    out
}

/// Integrated loudness (ITU-R BS.1770-4: K-weighting, 400 ms blocks, absolute and relative
/// gates).
pub fn lufs(signal: &[f32], sample_rate: u32) -> f32 {
    let sr = sample_rate as f32;
    let mut shelf = Biquad::new(Shape::HighShelf { db: 4.0 }, 1681.97, sr);
    let mut hp = Biquad::new(Shape::Highpass { q: 0.5003 }, 38.135, sr);
    let weighted: Vec<f32> = signal.iter().map(|&x| hp.run(shelf.run(x))).collect();
    let block = ((0.4 * sr) as usize).min(weighted.len()).max(1);
    let step = (block / 4).max(1);
    let powers: Vec<f64> = (0..=weighted.len().saturating_sub(block))
        .step_by(step)
        .map(|i| {
            weighted[i..i + block]
                .iter()
                .map(|&v| f64::from(v) * f64::from(v))
                .sum::<f64>()
                / block as f64
        })
        .collect();
    let loudness = |z: f64| -0.691 + 10.0 * z.max(1e-12).log10();
    let mean = |zs: &[f64]| zs.iter().sum::<f64>() / zs.len().max(1) as f64;
    let gated: Vec<f64> = powers
        .iter()
        .copied()
        .filter(|&z| loudness(z) > f64::from(SILENCE_LUFS))
        .collect();
    if gated.is_empty() {
        return SILENCE_LUFS;
    }
    let relative = loudness(mean(&gated)) - 10.0;
    let gated: Vec<f64> = gated
        .into_iter()
        .filter(|&z| loudness(z) > relative)
        .collect();
    (loudness(mean(&gated)) as f32).max(SILENCE_LUFS)
}

/// Reads a take's loudness, peak, tilt and sibilance.
pub fn measure(signal: &[f32], sample_rate: u32) -> Measures {
    let stft = Stft::new(2048, 1024);
    let mut power = vec![0f64; stft.bins()];
    stft.map(signal, |_, spectrum| {
        for (p, c) in power.iter_mut().zip(spectrum.iter()) {
            *p += f64::from(c.norm_sqr());
        }
    });
    let band = |lo: f32, hi: f32| -> f64 {
        (0..stft.bins())
            .filter(|&k| (lo..hi).contains(&stft.bin_hz(k, sample_rate)))
            .map(|k| power[k])
            .sum::<f64>()
            + 1e-12
    };
    let ratio_db = |num: f64, den: f64| (10.0 * (num / den).log10()) as f32;
    Measures {
        lufs: lufs(signal, sample_rate),
        peak_db: gain_to_db(signal.iter().fold(0f32, |m, v| m.max(v.abs()))),
        tilt_db: ratio_db(band(2000.0, 6000.0), band(150.0, 1000.0)),
        sibilance_db: ratio_db(band(5000.0, 10000.0), band(150.0, 5000.0)),
    }
}

/// The settings `preset` gives a take with measures `m` in a song at `bpm` (the delay is a
/// dotted eighth note).
pub fn settings(preset: ChainPreset, m: &Measures, bpm: f64) -> ChainSettings {
    let level = if m.lufs > SILENCE_LUFS {
        m.lufs
    } else {
        TARGET_LUFS
    };
    let beat_ms = (60_000.0 / if bpm > 0.0 { bpm } else { 120.0 }) as f32;
    // (comp ratio, low, presence, air, drive, reverb mix, reverb size, delay mix)
    let (ratio, low, presence, air, drive, reverb, size, delay) = match preset {
        ChainPreset::Natural => (2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.4, 0.0),
        ChainPreset::Auto | ChainPreset::Pop => (3.0, -1.0, 2.5, 3.0, 0.1, 0.15, 0.5, 0.08),
        ChainPreset::Rnb => (3.0, 1.5, 1.5, 2.5, 0.1, 0.25, 0.65, 0.1),
        ChainPreset::Rock => (4.0, -1.5, 4.0, 1.5, 0.45, 0.12, 0.4, 0.05),
        ChainPreset::HipHop => (4.0, 0.5, 3.0, 2.0, 0.25, 0.06, 0.3, 0.12),
        ChainPreset::Ballad => (2.5, 0.5, 1.5, 3.5, 0.0, 0.35, 0.85, 0.12),
    };
    let (tone, sibilance) = match preset {
        // Halfway towards a neutral tilt, through presence and air together.
        ChainPreset::Auto => (
            ((NEUTRAL_TILT_DB - m.tilt_db) * 0.5).clamp(-6.0, 6.0),
            m.sibilance_db - NEUTRAL_SIBILANCE_DB,
        ),
        _ => (0.0, 0.0),
    };
    // The compressor works the top 6 dB of the take's level; the de-esser's band threshold sits
    // under the take's level by the band's usual share, lower for sibilant takes.
    let comp_threshold = level + 6.0;
    let reduction_estimate = 6.0 * (1.0 - 1.0 / ratio) * 0.5;
    ChainSettings {
        highpass_hz: 90.0,
        deess_hz: 6000.0,
        deess_threshold_db: level - 8.0 - sibilance.max(0.0),
        deess_max_db: (6.0 + sibilance * 0.5).clamp(3.0, 12.0),
        comp_threshold_db: comp_threshold,
        comp_ratio: ratio,
        comp_makeup_db: 0.0,
        low_db: low,
        presence_db: presence + tone,
        air_db: air + tone,
        drive,
        reverb_mix: reverb,
        reverb_size: size,
        delay_mix: delay,
        delay_ms: beat_ms * 0.75,
        output_db: TARGET_LUFS - level + reduction_estimate,
    }
    .clamped()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    const SR: u32 = 44_100;

    fn sine(hz: f32, amp: f32, seconds: f32) -> Vec<f32> {
        (0..(SR as f32 * seconds) as usize)
            .map(|i| amp * (TAU * hz * i as f32 / SR as f32).sin())
            .collect()
    }

    fn rms_db(x: &[f32]) -> f32 {
        let tail = &x[x.len() / 2..];
        gain_to_db((tail.iter().map(|v| v * v).sum::<f32>() / tail.len() as f32).sqrt())
    }

    fn with(f: impl FnOnce(&mut ChainSettings)) -> ChainSettings {
        let mut s = ChainSettings::OFF;
        f(&mut s);
        s
    }

    fn gain_db(s: ChainSettings, hz: f32, amp: f32) -> f32 {
        let x = sine(hz, amp, 1.0);
        rms_db(&render(&x, SR, s)) - rms_db(&x)
    }

    #[test]
    fn off_is_identity() {
        let x = sine(440.0, 0.5, 0.5);
        assert_eq!(render(&x, SR, ChainSettings::OFF), x);
    }

    #[test]
    fn highpass_cuts_rumble_and_keeps_the_voice() {
        let s = with(|s| s.highpass_hz = 120.0);
        assert!(gain_db(s, 40.0, 0.3) < -30.0);
        assert!(gain_db(s, 1000.0, 0.3).abs() < 0.5);
    }

    #[test]
    fn deesser_pulls_down_loud_sibilance_only() {
        let s = with(|s| {
            s.deess_threshold_db = -30.0;
            s.deess_max_db = 9.0;
        });
        let cut = gain_db(s, 7500.0, 0.3);
        assert!((-9.5..-4.0).contains(&cut), "{cut}");
        assert!(gain_db(s, 300.0, 0.3).abs() < 0.3);
        // Quiet sibilance is under the threshold.
        assert!(gain_db(s, 7500.0, 0.01).abs() < 0.5);
    }

    #[test]
    fn compressor_reduces_above_the_threshold() {
        let s = with(|s| {
            s.comp_threshold_db = -20.0;
            s.comp_ratio = 4.0;
        });
        // 0.5 peak = −6 dBFS: 14 dB over, 10.5 dB reduction.
        let g = gain_db(s, 200.0, 0.5);
        assert!((-11.5..-9.0).contains(&g), "{g}");
        assert!(gain_db(s, 200.0, 0.05).abs() < 0.3);
    }

    #[test]
    fn eq_bands_boost_where_they_sit() {
        let s = with(|s| s.presence_db = 6.0);
        assert!((gain_db(s, PRESENCE_HZ, 0.1) - 6.0).abs() < 0.3);
        assert!(gain_db(s, 200.0, 0.1).abs() < 0.5);
        let s = with(|s| s.air_db = -6.0);
        assert!((gain_db(s, 16000.0, 0.1) + 6.0).abs() < 1.0);
        assert!(gain_db(s, 500.0, 0.1).abs() < 0.3);
        let s = with(|s| s.low_db = 4.0);
        assert!((gain_db(s, 60.0, 0.1) - 4.0).abs() < 0.5);
    }

    #[test]
    fn saturation_adds_odd_harmonics() {
        let s = with(|s| s.drive = 1.0);
        let x = sine(441.0, 0.8, 1.0);
        let y = render(&x, SR, s);
        let stft = Stft::new(4096, 4096);
        let spec = &stft.forward(&y)[3];
        let bin = |hz: f32| spec[(hz * 4096.0 / SR as f32).round() as usize].norm();
        assert!(bin(1323.0) > 0.05 * bin(441.0), "third harmonic");
    }

    #[test]
    fn reverb_and_delay_leave_tails() {
        let mut impulse = vec![0.0; SR as usize];
        impulse[0] = 1.0;
        let s = with(|s| {
            s.delay_mix = 0.5;
            s.delay_ms = 250.0;
        });
        let y = render(&impulse, SR, s);
        let at = (0.25 * SR as f32) as usize;
        assert!((y[at] - 0.5).abs() < 1e-6, "{}", y[at]);
        assert!(y[at * 2].abs() > 0.1, "feedback repeat");
        let s = with(|s| s.reverb_mix = 0.5);
        let y = render(&impulse, SR, s);
        let tail: f32 = y[SR as usize / 10..].iter().map(|v| v * v).sum();
        assert!(tail > 1e-4 && y.iter().all(|v| v.is_finite() && v.abs() < 2.0));
    }

    #[test]
    fn blocks_match_one_pass() {
        let x: Vec<f32> = sine(220.0, 0.6, 0.5)
            .iter()
            .zip(sine(7000.0, 0.2, 0.5))
            .map(|(a, b)| a + b)
            .collect();
        let s = settings(ChainPreset::Pop, &measure(&x, SR), 100.0);
        let whole = render(&x, SR, s);
        let mut chain = VocalChain::new(SR);
        chain.set(s);
        let mut blocks = x.clone();
        for b in blocks.chunks_mut(256) {
            chain.process(b);
        }
        assert_eq!(whole, blocks);
        chain.reset();
        let mut again = x.clone();
        chain.process(&mut again);
        assert_eq!(whole, again);
    }

    #[test]
    fn lufs_matches_bs1770_on_a_sine() {
        // A full-scale 1 kHz sine reads −3.01 LUFS.
        let l = lufs(&sine(1000.0, 0.1, 3.0), SR);
        assert!((l + 23.01).abs() < 0.3, "{l}");
        assert_eq!(lufs(&vec![0.0; SR as usize], SR), SILENCE_LUFS);
    }

    /// A sung-like tone: harmonics of 220 Hz falling as `1 / h^slope`, plus a hiss share.
    fn voice(slope: f32, hiss: f32, amp: f32) -> Vec<f32> {
        let mut s = 7u32;
        (0..SR as usize * 2)
            .map(|i| {
                let t = i as f32 / SR as f32;
                let tone: f32 = (1..40)
                    .map(|h| (TAU * 220.0 * h as f32 * t).sin() / (h as f32).powf(slope))
                    .sum();
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                let noise = (s >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0;
                amp * (0.3 * tone + hiss * noise)
            })
            .collect()
    }

    #[test]
    fn auto_brightens_dark_takes_and_tames_bright_ones() {
        let neutral = measure(&voice(1.0, 0.0, 0.3), SR);
        let bright = measure(&voice(0.5, 0.05, 0.3), SR);
        let dark = measure(&voice(2.0, 0.0, 0.3), SR);
        assert!(bright.tilt_db > neutral.tilt_db && neutral.tilt_db > dark.tilt_db);
        assert!(bright.sibilance_db > dark.sibilance_db);
        let auto = |m| settings(ChainPreset::Auto, &m, 120.0);
        let pop = settings(ChainPreset::Pop, &neutral, 120.0);
        assert!(auto(bright).air_db < pop.air_db, "bright: less air");
        assert!(auto(dark).air_db > pop.air_db, "dark: more air");
        assert!(auto(bright).deess_max_db > auto(dark).deess_max_db);
        assert!(auto(bright).deess_threshold_db < auto(dark).deess_threshold_db);
        // Other presets keep their own tone.
        assert_eq!(
            settings(ChainPreset::Pop, &bright, 120.0).air_db,
            pop.air_db
        );
    }

    #[test]
    fn every_preset_aims_quiet_and_loud_takes_at_the_target() {
        let quiet = measure(&voice(1.0, 0.0, 0.04), SR);
        let loud = measure(&voice(1.0, 0.0, 0.9), SR);
        assert!(
            quiet.lufs < -30.0 && loud.lufs > -15.0,
            "{quiet:?} {loud:?}"
        );
        for p in ChainPreset::ALL {
            let q = settings(p, &quiet, 120.0);
            let l = settings(p, &loud, 120.0);
            assert!(q.output_db > 10.0 && l.output_db < 3.0, "{p:?}");
            assert!(q.comp_threshold_db < l.comp_threshold_db, "{p:?}");
            // Rendered, both land near the target.
            for (m, s, amp) in [(quiet, q, 0.04), (loud, l, 0.9)] {
                let out = measure(&render(&voice(1.0, 0.0, amp), SR, s), SR);
                assert!(
                    (out.lufs - TARGET_LUFS).abs() < 4.0,
                    "{p:?} {m:?} -> {out:?}"
                );
            }
        }
        assert_eq!(settings(ChainPreset::Pop, &loud, 80.0).delay_ms, 562.5);
    }
}
