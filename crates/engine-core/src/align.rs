//! Fine alignment of a take to the reference (AGENTS.md §5): banded (Sakoe-Chiba) dynamic time
//! warping over the two pitch contours, after the coarse offset has put them on one timeline.

use crate::note::fold_octave;
use crate::pitch::PitchFrame;

/// Extra cost of a non-diagonal step, so silences (zero cost either way) stay unwarped.
const STEP_PENALTY: f32 = 0.05;

/// Step into a cell, kept for the backtrack.
#[derive(Clone, Copy)]
enum Step {
    Diagonal,
    /// Reference advances, take stays.
    Reference,
    /// Take advances, reference stays.
    Take,
}

/// For each reference frame, the take frame matched to it: the warping path between `reference`
/// and `take` (same length, same timeline) that stays within `radius` frames of the diagonal
/// and minimises the summed pitch distance.
pub fn warp(reference: &[PitchFrame], take: &[PitchFrame], radius: usize) -> Vec<usize> {
    let n = reference.len();
    assert_eq!(n, take.len(), "align after the coarse offset");
    if n == 0 {
        return Vec::new();
    }
    let width = 2 * radius + 1;
    // Band cell k of row i is take frame i + k - radius.
    let column = |i: usize, k: usize| (i + k).checked_sub(radius).filter(|j| *j < n);
    let mut steps = vec![Step::Diagonal; n * width];
    let mut previous = vec![f32::INFINITY; width];
    let mut current = vec![f32::INFINITY; width];
    for i in 0..n {
        for k in 0..width {
            current[k] = f32::INFINITY;
            let Some(j) = column(i, k) else { continue };
            let local = distance(reference[i], take[j]);
            if i == 0 && j == 0 {
                current[k] = local;
                continue;
            }
            let diagonal = if i > 0 { previous[k] } else { f32::INFINITY };
            let from_reference = if i > 0 && k + 1 < width {
                previous[k + 1]
            } else {
                f32::INFINITY
            };
            let from_take = if k > 0 { current[k - 1] } else { f32::INFINITY };
            let (best, step) = [
                (diagonal, Step::Diagonal),
                (from_reference + STEP_PENALTY, Step::Reference),
                (from_take + STEP_PENALTY, Step::Take),
            ]
            .into_iter()
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .expect("three candidates");
            current[k] = best + local;
            steps[i * width + k] = step;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    // Backtrack from (n-1, n-1); a reference frame keeps the earliest take frame it met.
    let mut matched = vec![0; n];
    let (mut i, mut k) = (n - 1, radius);
    loop {
        matched[i] = column(i, k).expect("path stays in the band");
        if i == 0 && k == radius {
            break;
        }
        match steps[i * width + k] {
            Step::Diagonal => i -= 1,
            Step::Reference => {
                i -= 1;
                k += 1;
            }
            Step::Take => k -= 1,
        }
    }
    matched
}

/// 0 when both agree (octave-folded) or both are silent, 1 at a semitone or more, or when only
/// one of them sings.
fn distance(a: PitchFrame, b: PitchFrame) -> f32 {
    match (a, b) {
        (PitchFrame::Voiced { midi: x }, PitchFrame::Voiced { midi: y }) => {
            (fold_octave(100.0 * (x - y)).abs() / 100.0).min(1.0)
        }
        (PitchFrame::Unvoiced, PitchFrame::Unvoiced) => 0.0,
        _ => 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn frames(pattern: &[(Option<f32>, usize)]) -> Vec<PitchFrame> {
        pattern
            .iter()
            .flat_map(|(midi, n)| {
                std::iter::repeat_n(
                    match midi {
                        Some(m) => PitchFrame::Voiced { midi: *m },
                        None => PitchFrame::Unvoiced,
                    },
                    *n,
                )
            })
            .collect()
    }

    #[test]
    fn identical_contours_warp_onto_the_diagonal() {
        let r = frames(&[(None, 10), (Some(60.0), 30), (None, 5), (Some(62.0), 20)]);
        assert_eq!(warp(&r, &r, 8), (0..r.len()).collect::<Vec<_>>());
    }

    #[test]
    fn a_late_note_is_matched_late() {
        let r = frames(&[(None, 20), (Some(60.0), 30), (None, 30)]);
        let t = frames(&[(None, 27), (Some(60.0), 30), (None, 23)]);
        let w = warp(&r, &t, 10);
        assert_eq!(w[20], 27, "note start");
        assert_eq!(w[49], 56, "note end");
        assert_eq!(w[79], 79);
    }
}
