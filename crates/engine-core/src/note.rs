//! Pitch unit conversions.

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

/// Fractional MIDI note number of a frequency (A4 = 440 Hz = 69).
pub fn hz_to_midi(hz: f32) -> f32 {
    69.0 + 12.0 * (hz / 440.0).log2()
}

pub fn midi_to_hz(midi: f32) -> f32 {
    440.0 * 2f32.powf((midi - 69.0) / 12.0)
}

/// Scientific pitch name of the nearest semitone, e.g. `A4`, `C#3`.
pub fn note_name(midi: f32) -> String {
    let n = midi.round() as i32;
    let name = pitch_class_name(n.rem_euclid(12) as u8);
    let octave = n.div_euclid(12) - 1;
    format!("{name}{octave}")
}

/// Name of a pitch class, 0 = C.
pub fn pitch_class_name(pitch_class: u8) -> &'static str {
    NOTE_NAMES[pitch_class as usize % 12]
}

/// Signed distance from `reference_hz` to `hz` in cents.
pub fn cents(hz: f32, reference_hz: f32) -> f32 {
    1200.0 * (hz / reference_hz).log2()
}

/// Folds a cents difference into [-600, 600) so octave errors count as unison.
pub fn fold_octave(cents: f32) -> f32 {
    (cents + 600.0).rem_euclid(1200.0) - 600.0
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn conversions_round_trip() {
        assert!((hz_to_midi(440.0) - 69.0).abs() < 1e-5);
        assert!((midi_to_hz(60.0) - 261.6256).abs() < 1e-3);
        assert_eq!(note_name(69.0), "A4");
        assert_eq!(note_name(61.2), "C#4");
        assert_eq!(note_name(23.4), "B0");
        assert!((cents(880.0, 440.0) - 1200.0).abs() < 1e-3);
        assert!((fold_octave(1210.0) - 10.0).abs() < 1e-3);
        assert!((fold_octave(-1190.0) - 10.0).abs() < 1e-3);
    }
}
