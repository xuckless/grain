//! Auto-tune target selection (AGENTS.md §7 steps 2–3), shared by the live monitor
//! ([`crate::psola::LiveTune`]), the offline renderer ([`render`]) and M4's pitch editor (§7.1
//! step 5): a pitch frame goes in, a correction in semitones comes out.
//!
//! [`Tuner`] runs one frame at a time and never allocates, so the live path can call it on the
//! audio thread; [`correction_curve`] runs it over a whole take.

use crate::key::Key;
use crate::melody::Note;
use crate::pitch::{PitchFrame, PitchTrack};
use crate::project::Project;
use crate::shift;
use crate::take::TakeError;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// The pitch classes notes may snap to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum Scale {
    /// Every semitone.
    Chromatic,
    /// The seven notes of a major or minor key (the song's detected key by default).
    Key { key: Key },
    /// Any set of pitch classes: bit `pc` set = pitch class `pc` allowed (0 = C).
    Custom { pitch_classes: u16 },
}

impl Scale {
    /// Bit `pc` set = pitch class `pc` allowed. Never empty (an empty custom set is chromatic).
    pub fn mask(self) -> u16 {
        let mask = match self {
            Scale::Chromatic => 0x0fff,
            Scale::Key { key } => (0..12u8)
                .filter(|&pc| key.contains(pc))
                .fold(0, |m, pc| m | 1 << pc),
            Scale::Custom { pitch_classes } => pitch_classes & 0x0fff,
        };
        if mask == 0 {
            0x0fff
        } else {
            mask
        }
    }

    /// The allowed note (whole MIDI number) nearest to `midi`.
    pub fn snap(self, midi: f32) -> f32 {
        let mask = self.mask();
        let base = midi.round() as i32;
        // The nearest allowed note is at most 6 semitones away (12 classes, ≥ 1 allowed).
        (0..=6)
            .flat_map(|d| [base - d, base + d])
            .filter(|n| mask & (1 << n.rem_euclid(12)) != 0)
            .min_by(|a, b| {
                (*a as f32 - midi)
                    .abs()
                    .total_cmp(&(*b as f32 - midi).abs())
            })
            .map_or(midi, |n| n as f32)
    }
}

/// What a sung note is pulled towards.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum TuneMode {
    /// The nearest note of the scale.
    Scale,
    /// The reference melody's note at that moment (in the singer's octave); the scale where the
    /// melody rests.
    FollowMelody,
}

/// The auto-tune controls (AGENTS.md §7 step 5).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TuneSettings {
    pub mode: TuneMode,
    pub scale: Scale,
    /// Time constant of the correction, in ms: 0 snaps instantly (the hard effect); 50–400
    /// sounds natural.
    pub retune_ms: f32,
    /// 0–1: how much longer sustained notes take to correct, keeping their natural vibrato.
    pub humanize: f32,
    /// Formant shift in semitones (within ±[`MAX_FORMANT_SEMITONES`]); 0 keeps the voice's
    /// character.
    pub formant_semitones: f32,
    /// 0 = dry, 1 = fully corrected.
    pub mix: f32,
}

/// Natural-sounding defaults, chromatic.
impl Default for TuneSettings {
    fn default() -> Self {
        Self {
            mode: TuneMode::Scale,
            scale: Scale::Chromatic,
            retune_ms: 40.0,
            humanize: 0.3,
            formant_semitones: 0.0,
            mix: 1.0,
        }
    }
}

impl TuneSettings {
    /// The defaults for a song in `key`.
    pub fn for_key(key: Key) -> Self {
        Self {
            scale: Scale::Key { key },
            ..Self::default()
        }
    }

    /// The same settings with every control in range.
    pub fn clamped(self) -> Self {
        Self {
            retune_ms: self.retune_ms.clamp(0.0, MAX_RETUNE_MS),
            humanize: self.humanize.clamp(0.0, 1.0),
            formant_semitones: self
                .formant_semitones
                .clamp(-MAX_FORMANT_SEMITONES, MAX_FORMANT_SEMITONES),
            mix: self.mix.clamp(0.0, 1.0),
            ..self
        }
    }

    /// Pitch factor of the formant shift.
    pub fn formant_factor(self) -> f32 {
        2f32.powf(self.clamped().formant_semitones / 12.0)
    }
}

pub const MAX_RETUNE_MS: f32 = 1000.0;
pub const MAX_FORMANT_SEMITONES: f32 = 5.0;
/// A note must be this much closer (semitones) to a new target before the tuner switches to it,
/// so a pitch hovering at a boundary doesn't flicker between two notes (§7 step 2).
pub const HYSTERESIS: f32 = 0.15;
/// Extra time constant humanize adds to a fully sustained note.
const HUMANIZE_MS: f32 = 400.0;
/// How long a note must be held before humanize applies in full.
const SUSTAIN_SECONDS: f32 = 0.25;

/// What the tuner is pulling towards.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Target {
    /// Unvoiced: nothing to correct.
    Free,
    /// Pulling towards `note` (fractional MIDI), held for `held` seconds. `melody`: the
    /// reference note it came from (follow-melody mode).
    Holding {
        note: f32,
        held: f32,
        melody: Option<f32>,
    },
}

/// One frame of the tuner's output.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Correction {
    /// The note being pulled towards (`None` when unvoiced).
    pub target: Option<f32>,
    /// Pitch change to apply now, in semitones (0 when unvoiced).
    pub semitones: f32,
}

/// Frame-by-frame target selection and retune smoothing. Allocation-free.
pub struct Tuner {
    settings: TuneSettings,
    hop_seconds: f32,
    target: Target,
    /// The smoothed correction, in semitones.
    smoothed: f32,
}

impl Tuner {
    /// `hop_seconds`: time between the frames passed to [`Tuner::next`].
    pub fn new(settings: TuneSettings, hop_seconds: f32) -> Self {
        Self {
            settings: settings.clamped(),
            hop_seconds,
            target: Target::Free,
            smoothed: 0.0,
        }
    }

    pub fn set_settings(&mut self, settings: TuneSettings) {
        self.settings = settings.clamped();
    }

    pub fn settings(&self) -> TuneSettings {
        self.settings
    }

    /// The next frame's correction. `melody`: the reference note sounding now (fractional MIDI),
    /// used in follow-melody mode.
    pub fn next(&mut self, frame: PitchFrame, melody: Option<f32>) -> Correction {
        let PitchFrame::Voiced { midi } = frame else {
            self.target = Target::Free;
            self.smoothed = 0.0;
            return Correction {
                target: None,
                semitones: 0.0,
            };
        };
        let melody = match self.settings.mode {
            TuneMode::FollowMelody => melody,
            TuneMode::Scale => None,
        };
        let candidate = match melody {
            Some(m) => nearest_octave(m, midi),
            None => self.settings.scale.snap(midi),
        };
        let (note, held) = match self.target {
            Target::Holding {
                note,
                held,
                melody: was,
            } if note == candidate
                || (was == melody
                    && (midi - candidate).abs() + HYSTERESIS >= (midi - note).abs()) =>
            {
                (note, held + self.hop_seconds)
            }
            _ => (candidate, 0.0),
        };
        self.target = Target::Holding { note, held, melody };

        let sustain = (held / SUSTAIN_SECONDS).min(1.0);
        let tau =
            (self.settings.retune_ms + self.settings.humanize * HUMANIZE_MS * sustain) / 1000.0;
        let alpha = if tau <= 0.0 {
            1.0
        } else {
            1.0 - (-self.hop_seconds / tau).exp()
        };
        self.smoothed += alpha * ((note - midi) - self.smoothed);
        Correction {
            target: Some(note),
            semitones: self.smoothed,
        }
    }
}

/// The octave of `note` nearest to `midi`.
fn nearest_octave(note: f32, midi: f32) -> f32 {
    note + 12.0 * ((midi - note) / 12.0).round()
}

/// The reference note sounding at `seconds`, if any (`notes` sorted by start).
pub fn melody_at(notes: &[Note], seconds: f64) -> Option<f32> {
    let i = notes.partition_point(|n| n.start <= seconds);
    let note = notes.get(i.checked_sub(1)?)?;
    (seconds < note.end).then_some(f32::from(note.midi))
}

/// The tuner run over a whole pitch track: one [`Correction`] per frame.
pub fn correction_curve(
    track: &PitchTrack,
    notes: &[Note],
    settings: TuneSettings,
) -> Vec<Correction> {
    let mut tuner = Tuner::new(settings, track.hop_seconds as f32);
    track
        .frames
        .iter()
        .enumerate()
        .map(|(i, f)| tuner.next(*f, melody_at(notes, track.time(i))))
        .collect()
}

/// What auto-tune would do to a take, drawn before rendering: per frame of the take's pitch,
/// the sung pitch, the note it is pulled to, and the pitch after correction (fractional MIDI,
/// `None` when unvoiced).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct TunePreview {
    pub hop_seconds: f64,
    pub sung: Vec<Option<f32>>,
    pub target: Vec<Option<f32>>,
    pub corrected: Vec<Option<f32>>,
}

fn take_pitch(project: &Project, id: u32) -> Result<&PitchTrack, TakeError> {
    project
        .takes
        .iter()
        .find(|t| t.id == id)
        .map(|t| &t.pitch)
        .ok_or(TakeError::Unknown(id))
}

/// The preview of tuning take `id` (from its stored pitch; no audio needed).
pub fn preview(
    project: &Project,
    id: u32,
    settings: TuneSettings,
) -> Result<TunePreview, TakeError> {
    let track = take_pitch(project, id)?;
    let curve = correction_curve(track, &project.analysis.reference_notes, settings);
    let mix = settings.clamped().mix;
    let sung: Vec<Option<f32>> = track
        .frames
        .iter()
        .map(|f| match f {
            PitchFrame::Voiced { midi } => Some(*midi),
            PitchFrame::Unvoiced => None,
        })
        .collect();
    Ok(TunePreview {
        hop_seconds: track.hop_seconds,
        corrected: sung
            .iter()
            .zip(&curve)
            .map(|(s, c)| s.map(|m| m + mix * c.semitones))
            .collect(),
        target: curve.iter().map(|c| c.target).collect(),
        sung,
    })
}

/// Renders take `id` tuned (offline, high quality): `timeline` is the take's audio on the song
/// timeline at the project rate. Returns the new take's timeline audio; the original is left as
/// it is (AGENTS.md §7: non-destructive).
pub fn render(
    project: &Project,
    id: u32,
    timeline: &[f32],
    settings: TuneSettings,
) -> Result<Vec<f32>, TakeError> {
    let track = take_pitch(project, id)?;
    let settings = settings.clamped();
    let curve: Vec<f32> = correction_curve(track, &project.analysis.reference_notes, settings)
        .iter()
        .map(|c| c.semitones)
        .collect();
    let wet = shift::shift(
        timeline,
        project.sample_rate,
        &curve,
        track.hop_seconds,
        settings.formant_semitones,
    );
    Ok(timeline
        .iter()
        .zip(&wet)
        .map(|(d, w)| (1.0 - settings.mix) * d + settings.mix * w)
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::Mode;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    const C_MAJOR: Key = Key {
        tonic: 0,
        mode: Mode::Major,
    };
    const HOP: f32 = 0.01;

    fn voiced(midi: f32) -> PitchFrame {
        PitchFrame::Voiced { midi }
    }

    fn hard(scale: Scale) -> TuneSettings {
        TuneSettings {
            mode: TuneMode::Scale,
            scale,
            retune_ms: 0.0,
            humanize: 0.0,
            formant_semitones: 0.0,
            mix: 1.0,
        }
    }

    #[test]
    fn scales_snap_to_their_notes() {
        let c_major = Scale::Key { key: C_MAJOR };
        assert_eq!(c_major.mask(), 0b1010_1011_0101);
        // C#4 (61) is not in C major: 60 and 62 are equally near; 61.2 goes up.
        assert_eq!(c_major.snap(61.2), 62.0);
        assert_eq!(c_major.snap(60.9), 60.0);
        assert_eq!(Scale::Chromatic.snap(60.6), 61.0);
        let a_minor_pentatonic = Scale::Custom {
            pitch_classes: (1 << 9) | 1 | (1 << 2) | (1 << 4) | (1 << 7),
        };
        assert_eq!(a_minor_pentatonic.snap(65.0), 64.0); // F → E
        assert_eq!(a_minor_pentatonic.snap(66.0), 67.0); // F# → G
        assert_eq!(Scale::Custom { pitch_classes: 0 }.snap(60.6), 61.0);
    }

    #[test]
    fn hard_retune_snaps_at_once() {
        let mut t = Tuner::new(hard(Scale::Chromatic), HOP);
        let c = t.next(voiced(60.3), None);
        assert_eq!(c.target, Some(60.0));
        assert!((c.semitones + 0.3).abs() < 1e-5);
        let c = t.next(PitchFrame::Unvoiced, None);
        assert_eq!(c.target, None);
        assert_eq!(c.semitones, 0.0);
    }

    #[test]
    fn retune_speed_is_a_one_pole_step_response() {
        for retune_ms in [50.0f32, 200.0] {
            let mut t = Tuner::new(
                TuneSettings {
                    retune_ms,
                    ..hard(Scale::Chromatic)
                },
                HOP,
            );
            // A steady note 40 cents sharp: the correction approaches −0.4 st.
            let curve: Vec<f32> = (0..200)
                .map(|_| t.next(voiced(60.4), None).semitones)
                .collect();
            // After one time constant, 1 − 1/e of the way (to within a frame).
            let frames = (retune_ms / 1000.0 / HOP).round() as usize;
            let expected = -0.4 * (1.0 - (-1.0f32).exp());
            assert!(
                (curve[frames - 1] - expected).abs() < 0.03,
                "{retune_ms} ms: {} vs {expected}",
                curve[frames - 1]
            );
            assert!(curve.windows(2).all(|w| w[1] <= w[0] + 1e-6), "monotonic");
            assert!((curve[199] + 0.4).abs() < 0.01);
        }
    }

    #[test]
    fn hysteresis_stops_flicker_at_a_note_boundary() {
        let mut t = Tuner::new(hard(Scale::Chromatic), HOP);
        // Hovering around the 60/61 boundary (60.5), ±0.1 st.
        let targets: Vec<f32> = (0..100)
            .map(|i| {
                let midi = 60.45 + 0.1 * ((i as f32) * 0.7).sin();
                t.next(voiced(midi), None).target.unwrap()
            })
            .collect();
        assert!(targets.iter().all(|&n| n == 60.0), "{targets:?}");
        // A clear move to the next note does switch.
        assert_eq!(t.next(voiced(60.8), None).target, Some(61.0));
        // Without hysteresis, the same hover would flicker.
        let flips = (0..100)
            .map(|i| Scale::Chromatic.snap(60.45 + 0.1 * ((i as f32) * 0.7).sin()))
            .collect::<Vec<_>>()
            .windows(2)
            .filter(|w| w[0] != w[1])
            .count();
        assert!(flips > 5);
    }

    #[test]
    fn humanize_keeps_vibrato_on_sustained_notes() {
        let vibrato = |humanize: f32| {
            let mut t = Tuner::new(
                TuneSettings {
                    humanize,
                    ..hard(Scale::Chromatic)
                },
                HOP,
            );
            // 1.5 s of A4 with a 5.5 Hz, ±30 cent vibrato.
            let out: Vec<f32> = (0..150)
                .map(|i| {
                    let midi =
                        69.0 + 0.3 * (2.0 * std::f32::consts::PI * 5.5 * i as f32 * HOP).sin();
                    midi + t.next(voiced(midi), None).semitones
                })
                .collect();
            // Peak-to-peak of the corrected pitch over the last second.
            let tail = &out[50..];
            tail.iter().cloned().fold(f32::MIN, f32::max)
                - tail.iter().cloned().fold(f32::MAX, f32::min)
        };
        let (plain, humanized) = (vibrato(0.0), vibrato(1.0));
        assert!(plain < 0.05, "hard retune flattens the vibrato: {plain}");
        assert!(
            humanized > 2.0 * plain,
            "humanized {humanized} vs plain {plain}"
        );
        assert!(
            humanized > 0.35,
            "most of the 0.6 st vibrato remains: {humanized}"
        );
    }

    #[test]
    fn follow_melody_snaps_to_the_reference_in_the_singers_octave() {
        let settings = TuneSettings {
            mode: TuneMode::FollowMelody,
            ..hard(Scale::Key { key: C_MAJOR })
        };
        let notes = [
            Note {
                start: 0.0,
                end: 0.5,
                midi: 64,
            },
            Note {
                start: 0.5,
                end: 1.0,
                midi: 65,
            },
        ];
        assert_eq!(melody_at(&notes, 0.25), Some(64.0));
        assert_eq!(melody_at(&notes, 0.75), Some(65.0));
        assert_eq!(melody_at(&notes, 1.5), None);
        // Sung an octave low and flat: 51.4 → E3 (52), then F3 (53) when the melody moves.
        let track = PitchTrack {
            hop_seconds: 0.01,
            frames: (0..150).map(|_| voiced(51.4)).collect(),
        };
        let curve = correction_curve(&track, &notes, settings);
        assert_eq!(curve[10].target, Some(52.0));
        assert_eq!(curve[60].target, Some(53.0));
        // Where the melody rests, the scale applies: the nearest C-major note to 51.4 is E3 (52).
        assert_eq!(curve[120].target, Some(52.0));
        // Scale mode ignores the melody.
        let scale = correction_curve(
            &track,
            &notes,
            TuneSettings {
                mode: TuneMode::Scale,
                ..settings
            },
        );
        assert_eq!(scale[60].target, Some(52.0));
    }
}
