//! The pitch editor (AGENTS.md §7.1): note-based, non-destructive editing of a monophonic vocal,
//! Melodyne-style but simpler.
//!
//! A [`PitchEdit`] is the edit list kept with the project: one [`EditNote`] per sung note, each
//! holding its analysed span and centre plus what the user changed (target pitch, drift and
//! vibrato amounts, glide, formant, timing). [`apply`] performs one [`EditOp`] (the UI keeps
//! undo/redo as snapshots of the list). [`curves`] turns the list into the per-frame pitch and
//! formant corrections and the [`TimeMap`]; [`render`] runs them through the shared offline
//! shifter ([`crate::shift`]) and time warp ([`crate::stretch`]), re-rendering only what changed.
//!
//! Notes come from §6.2's segmentation ([`crate::melody::segment`]) and snapping from the
//! auto-tune [`Scale`] ([`crate::tune`]), so the editor and auto-tune share one implementation.

use crate::key::Key;
use crate::melody;
use crate::pitch::{PitchFrame, PitchTrack};
use crate::project::Project;
use crate::shift;
use crate::stretch::{self, TimeMap};
use crate::tune::{Scale, TuneSettings};
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// Window of the moving average that separates a note's slow drift from its vibrato.
const DRIFT_SECONDS: f64 = 0.15;
/// Shortest note a split, move or stretch may leave.
pub const MIN_NOTE_SECONDS: f64 = 0.05;
/// Stretch limits (output length / sung length).
pub const MIN_STRETCH: f64 = 0.5;
pub const MAX_STRETCH: f64 = 2.0;
/// Modulation (vibrato) scale limits.
pub const MAX_MODULATION: f32 = 2.0;
pub const MAX_TRANSITION_SECONDS: f32 = 0.5;
pub const MAX_FORMANT_SEMITONES: f32 = crate::tune::MAX_FORMANT_SEMITONES;
/// Audio kept around each re-rendered stretch, and the crossfade into it.
const RENDER_MARGIN_SECONDS: f64 = 0.1;
const RENDER_FADE_SECONDS: f64 = 0.02;

/// What is being edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum EditSource {
    /// A take of the loaded song.
    Take { id: u32 },
    /// The song's separated vocal (the reference).
    VocalStem,
}

/// One note of the edit list. Times are song seconds; pitches fractional MIDI.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EditNote {
    /// Stable id (splits get fresh ones).
    pub id: u32,
    /// The note as sung.
    pub start: f64,
    pub end: f64,
    /// Median sung pitch.
    pub centre: f32,
    /// Where the note's centre goes (`centre` = unchanged).
    pub pitch: f32,
    /// Share of the slow drift kept (1 = as sung, 0 = flat).
    pub drift: f32,
    /// Vibrato scale (0 = none, 1 = as sung, 2 = double).
    pub modulation: f32,
    /// Glide from the previous note's correction into this one's, seconds (0 = a step).
    pub transition: f32,
    /// Formant shift, semitones.
    pub formant: f32,
    /// Where the note plays (moved or stretched); `start`/`end` when untouched.
    pub out_start: f64,
    pub out_end: f64,
}

impl EditNote {
    fn new(id: u32, start: f64, end: f64, centre: f32) -> Self {
        Self {
            id,
            start,
            end,
            centre,
            pitch: centre,
            drift: 1.0,
            modulation: 1.0,
            transition: 0.0,
            formant: 0.0,
            out_start: start,
            out_end: end,
        }
    }

    /// Whether anything about the note was changed.
    pub fn edited(&self) -> bool {
        *self != Self::new(self.id, self.start, self.end, self.centre)
    }

    fn reset(&mut self) {
        *self = Self::new(self.id, self.start, self.end, self.centre);
    }

    fn offset(&self) -> f32 {
        self.pitch - self.centre
    }
}

/// The edit list kept with the project for one source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PitchEdit {
    pub source: EditSource,
    /// Scale that snapping and "Correct pitch" use.
    pub scale: Scale,
    /// Sorted by `start`, not overlapping.
    pub notes: Vec<EditNote>,
}

/// What the editor draws: the edit list plus the source's pitch and loudness per frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct PitchEditView {
    pub edit: PitchEdit,
    pub hop_seconds: f64,
    /// The pitch as sung (fractional MIDI, `None` when unvoiced).
    pub sung: Vec<Option<f32>>,
    /// The pitch after the edits, on the output timeline.
    pub edited: Vec<Option<f32>>,
    /// Loudness per frame, 0–1 (the loudest frame is 1).
    pub amplitude: Vec<f32>,
    /// Pitch classes of the edit's scale (bit 0 = C), for the scale lines.
    pub scale_mask: u16,
}

/// How a pitch drag lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub enum Snap {
    /// Exactly where dropped (cents).
    Free,
    /// The nearest semitone.
    Semitone,
    /// The nearest note of the edit's scale.
    Scale,
}

/// One edit. Notes are named by id; ops on unknown ids do nothing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum EditOp {
    /// Moves the notes' centres by `semitones` (relative to where they are), then snaps each.
    MovePitch {
        ids: Vec<u32>,
        semitones: f32,
        snap: Snap,
    },
    /// "Pitch centre" (`centre`, 0–1: share of the way to the nearest scale note) and/or
    /// "Pitch drift" (`drift`, 0–1: share of the drift flattened). "Correct pitch" is both.
    Correct {
        ids: Vec<u32>,
        centre: Option<f32>,
        drift: Option<f32>,
    },
    SetModulation {
        ids: Vec<u32>,
        modulation: f32,
    },
    SetTransition {
        ids: Vec<u32>,
        seconds: f32,
    },
    SetFormant {
        ids: Vec<u32>,
        semitones: f32,
    },
    /// Moves a note in time (keeping its length) to start at `out_start`.
    MoveTime {
        id: u32,
        out_start: f64,
    },
    /// Stretches a note to `[out_start, out_end]`.
    Stretch {
        id: u32,
        out_start: f64,
        out_end: f64,
    },
    /// Splits a note at song time `at` (on its sung timeline).
    Split {
        id: u32,
        at: f64,
    },
    /// Merges a note with the next one.
    Merge {
        id: u32,
    },
    /// Undoes everything done to the notes.
    Reset {
        ids: Vec<u32>,
    },
    /// Changes the scale used for snapping.
    SetScale {
        scale: Scale,
    },
}

/// The edit list after a change, and the pitch it now gives (for redrawing).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct EditUpdate {
    pub edit: PitchEdit,
    pub edited: Vec<Option<f32>>,
    /// Pitch classes of the edit's scale (bit 0 = C), for the scale lines.
    pub scale_mask: u16,
}

#[derive(Debug, thiserror::Error)]
pub enum EditError {
    #[error("no take {0}")]
    UnknownTake(u32),
    #[error("this song has no vocal melody to edit")]
    NoVocal,
}

/// The pitch track of `source` in `project`.
pub fn source_track(project: &Project, source: EditSource) -> Result<&PitchTrack, EditError> {
    match source {
        EditSource::VocalStem if !project.analysis.reference_notes.is_empty() => {
            Ok(&project.analysis.reference_pitch)
        }
        EditSource::VocalStem => Err(EditError::NoVocal),
        EditSource::Take { id } => project
            .takes
            .iter()
            .find(|t| t.id == id)
            .map(|t| &t.pitch)
            .ok_or(EditError::UnknownTake(id)),
    }
}

/// The song's key (snapping and segmentation), or C major when the analysis found none.
pub fn song_key(project: &Project) -> Key {
    project.analysis.keys.first().map_or(
        Key {
            tonic: 0,
            mode: crate::key::Mode::Major,
        },
        |k| k.key,
    )
}

/// The saved edit list of `source`, or a fresh analysis (snapping to auto-tune's scale).
pub fn edit_of(project: &Project, source: EditSource) -> Result<PitchEdit, EditError> {
    if let Some(e) = project.pitch_edits.iter().find(|e| e.source == source) {
        return Ok(e.clone());
    }
    let track = source_track(project, source)?;
    Ok(analyze(
        source,
        track,
        song_key(project),
        project.tune.scale,
    ))
}

/// Opens `source` in the editor (`audio`: its mono audio on the song timeline).
pub fn open(
    project: &Project,
    source: EditSource,
    audio: &[f32],
) -> Result<PitchEditView, EditError> {
    let edit = edit_of(project, source)?;
    Ok(view(
        edit,
        source_track(project, source)?,
        audio,
        project.sample_rate,
    ))
}

/// Keeps `edit` with the project (replacing that source's list) and returns the redraw.
pub fn store(project: &mut Project, edit: PitchEdit) -> Result<EditUpdate, EditError> {
    let edited = edited_pitch(&edit, source_track(project, edit.source)?);
    match project
        .pitch_edits
        .iter_mut()
        .find(|e| e.source == edit.source)
    {
        Some(e) => *e = edit.clone(),
        None => project.pitch_edits.push(edit.clone()),
    }
    Ok(EditUpdate {
        scale_mask: edit.scale.mask(),
        edit,
        edited,
    })
}

/// Applies `op` to `edit`, keeps the result with the project and returns the redraw.
pub fn apply_op(
    project: &mut Project,
    mut edit: PitchEdit,
    op: &EditOp,
) -> Result<EditUpdate, EditError> {
    let track = source_track(project, edit.source)?;
    apply(&mut edit, op, track);
    store(project, edit)
}

/// Seeds take `take`'s edit list from auto-tune's settings and keeps it with the project.
pub fn seed_from_tune(
    project: &mut Project,
    take: u32,
    settings: TuneSettings,
) -> Result<EditUpdate, EditError> {
    let source = EditSource::Take { id: take };
    let edit = from_tune(
        source,
        source_track(project, source)?,
        song_key(project),
        &project.analysis.reference_notes,
        settings,
    );
    store(project, edit)
}

/// Renders the saved (or fresh) edit list of `source` over `audio` (its timeline audio).
pub fn render_source(
    project: &Project,
    source: EditSource,
    audio: &[f32],
) -> Result<Vec<f32>, EditError> {
    let edit = edit_of(project, source)?;
    Ok(render(
        &edit,
        source_track(project, source)?,
        audio,
        project.sample_rate,
    ))
}

/// Sung pitch of a frame, if voiced.
fn voiced(f: &PitchFrame) -> Option<f32> {
    match f {
        PitchFrame::Voiced { midi } => Some(*midi),
        PitchFrame::Unvoiced => None,
    }
}

/// Median voiced pitch of the frames in `[start, end)`, if any.
fn median_pitch(track: &PitchTrack, start: f64, end: f64) -> Option<f32> {
    let mut v: Vec<f32> = frames_in(track, start, end)
        .filter_map(|i| voiced(&track.frames[i]))
        .collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(f32::total_cmp);
    Some(v[v.len() / 2])
}

/// Frame indices of `track` whose time is in `[start, end)`.
fn frames_in(track: &PitchTrack, start: f64, end: f64) -> impl Iterator<Item = usize> {
    let hop = track.hop_seconds;
    let a = (start / hop).ceil().max(0.0) as usize;
    let b = ((end / hop).ceil().max(0.0) as usize).min(track.frames.len());
    a..b.max(a)
}

/// A fresh edit list for `track`: one note per segmented note (§6.2), centred on its median.
pub fn analyze(source: EditSource, track: &PitchTrack, key: Key, scale: Scale) -> PitchEdit {
    let notes = melody::segment(track, key)
        .iter()
        .enumerate()
        .filter_map(|(i, n)| {
            let centre = median_pitch(track, n.start, n.end)?;
            Some(EditNote::new(i as u32 + 1, n.start, n.end, centre))
        })
        .collect();
    PitchEdit {
        source,
        scale,
        notes,
    }
}

/// Loudness per pitch frame (RMS over one hop, normalized so the loudest frame is 1).
pub fn amplitude(track: &PitchTrack, audio: &[f32], sample_rate: u32) -> Vec<f32> {
    let hop = (track.hop_seconds * f64::from(sample_rate)) as usize;
    let rms: Vec<f32> = (0..track.frames.len())
        .map(|i| {
            let a = (i * hop).saturating_sub(hop / 2).min(audio.len());
            let b = (a + hop).min(audio.len());
            let s = &audio[a..b];
            if s.is_empty() {
                0.0
            } else {
                (s.iter().map(|v| v * v).sum::<f32>() / s.len() as f32).sqrt()
            }
        })
        .collect();
    let peak = rms.iter().copied().fold(0.0f32, f32::max).max(1e-9);
    rms.iter().map(|v| v / peak).collect()
}

/// The view of `edit` over `track` and its source audio.
pub fn view(edit: PitchEdit, track: &PitchTrack, audio: &[f32], sample_rate: u32) -> PitchEditView {
    PitchEditView {
        hop_seconds: track.hop_seconds,
        sung: track.frames.iter().map(voiced).collect(),
        edited: edited_pitch(&edit, track),
        amplitude: amplitude(track, audio, sample_rate),
        scale_mask: edit.scale.mask(),
        edit,
    }
}

/// Applies `op` to `edit` (`track`: the source's pitch, for re-centring split/merged notes).
pub fn apply(edit: &mut PitchEdit, op: &EditOp, track: &PitchTrack) {
    let scale = edit.scale;
    let each = |edit: &mut PitchEdit, ids: &[u32], f: &mut dyn FnMut(&mut EditNote)| {
        edit.notes
            .iter_mut()
            .filter(|n| ids.contains(&n.id))
            .for_each(&mut *f);
    };
    match op {
        EditOp::MovePitch {
            ids,
            semitones,
            snap,
        } => each(edit, ids, &mut |n| {
            let moved = n.pitch + semitones;
            n.pitch = match snap {
                Snap::Free => moved,
                Snap::Semitone => moved.round(),
                Snap::Scale => scale.snap(moved),
            };
        }),
        EditOp::Correct { ids, centre, drift } => each(edit, ids, &mut |n| {
            if let Some(c) = centre {
                let target = scale.snap(n.centre);
                n.pitch = n.centre + c.clamp(0.0, 1.0) * (target - n.centre);
            }
            if let Some(d) = drift {
                n.drift = 1.0 - d.clamp(0.0, 1.0);
            }
        }),
        EditOp::SetModulation { ids, modulation } => each(edit, ids, &mut |n| {
            n.modulation = modulation.clamp(0.0, MAX_MODULATION)
        }),
        EditOp::SetTransition { ids, seconds } => each(edit, ids, &mut |n| {
            n.transition = seconds.clamp(0.0, MAX_TRANSITION_SECONDS)
        }),
        EditOp::SetFormant { ids, semitones } => each(edit, ids, &mut |n| {
            n.formant = semitones.clamp(-MAX_FORMANT_SEMITONES, MAX_FORMANT_SEMITONES)
        }),
        EditOp::MoveTime { id, out_start } => {
            if let Some(i) = index(edit, *id) {
                let n = edit.notes[i];
                let len = n.out_end - n.out_start;
                let (lo, hi) = neighbours(edit, i);
                let start = out_start.clamp(lo, (hi - len).max(lo));
                edit.notes[i].out_start = start;
                edit.notes[i].out_end = start + len;
            }
        }
        EditOp::Stretch {
            id,
            out_start,
            out_end,
        } => {
            if let Some(i) = index(edit, *id) {
                let n = edit.notes[i];
                let sung = n.end - n.start;
                let (lo, hi) = neighbours(edit, i);
                let start = out_start.clamp(lo, hi - MIN_NOTE_SECONDS);
                let len = (out_end - start)
                    .clamp(sung * MIN_STRETCH, sung * MAX_STRETCH)
                    .max(MIN_NOTE_SECONDS)
                    .min(hi - start);
                edit.notes[i].out_start = start;
                edit.notes[i].out_end = start + len;
            }
        }
        EditOp::Split { id, at } => split(edit, *id, *at, track),
        EditOp::Merge { id } => merge(edit, *id, track),
        EditOp::Reset { ids } => each(edit, ids, &mut |n| n.reset()),
        EditOp::SetScale { scale } => edit.scale = *scale,
    }
}

fn index(edit: &PitchEdit, id: u32) -> Option<usize> {
    edit.notes.iter().position(|n| n.id == id)
}

/// The output-time room note `i` may occupy: between its neighbours' output spans.
fn neighbours(edit: &PitchEdit, i: usize) -> (f64, f64) {
    let lo = i.checked_sub(1).map_or(0.0, |j| edit.notes[j].out_end);
    let hi = edit.notes.get(i + 1).map_or(f64::MAX, |n| n.out_start);
    (lo, hi)
}

fn split(edit: &mut PitchEdit, id: u32, at: f64, track: &PitchTrack) {
    let Some(i) = index(edit, id) else { return };
    let n = edit.notes[i];
    if at - n.start < MIN_NOTE_SECONDS || n.end - at < MIN_NOTE_SECONDS {
        return;
    }
    let next_id = edit.notes.iter().map(|n| n.id).max().unwrap_or(0) + 1;
    // Both halves keep the note's edits; output times split in proportion.
    let out_at = n.out_start + (at - n.start) / (n.end - n.start) * (n.out_end - n.out_start);
    let recentre = |mut half: EditNote, start: f64, end: f64| {
        let offset = half.offset();
        half.start = start;
        half.end = end;
        half.centre = median_pitch(track, start, end).unwrap_or(half.centre);
        half.pitch = half.centre + offset;
        half
    };
    let mut first = recentre(n, n.start, at);
    first.out_end = out_at;
    let mut second = recentre(EditNote { id: next_id, ..n }, at, n.end);
    second.out_start = out_at;
    second.transition = 0.0;
    edit.notes[i] = first;
    edit.notes.insert(i + 1, second);
}

fn merge(edit: &mut PitchEdit, id: u32, track: &PitchTrack) {
    let Some(i) = index(edit, id) else { return };
    let Some(next) = edit.notes.get(i + 1).copied() else {
        return;
    };
    let n = &mut edit.notes[i];
    let offset = n.offset();
    n.end = next.end;
    n.out_end = next.out_end;
    n.centre = median_pitch(track, n.start, n.end).unwrap_or(n.centre);
    n.pitch = n.centre + offset;
    edit.notes.remove(i + 1);
}

/// The per-frame corrections an edit list asks for, on the source's (sung) timeline, and the
/// map from output time to sung time.
pub struct Curves {
    /// Pitch change per frame, semitones.
    pub pitch: Vec<f32>,
    /// Formant change per frame, semitones.
    pub formant: Vec<f32>,
    pub time: TimeMap,
}

/// The moving average of the voiced pitch around frame `i` within `[a, b)` (the note's drift).
fn drift_at(track: &PitchTrack, i: usize, a: usize, b: usize) -> Option<f32> {
    let half = (DRIFT_SECONDS / 2.0 / track.hop_seconds).round() as usize;
    let lo = i.saturating_sub(half).max(a);
    let hi = (i + half + 1).min(b);
    let v: Vec<f32> = (lo..hi).filter_map(|j| voiced(&track.frames[j])).collect();
    (!v.is_empty()).then(|| v.iter().sum::<f32>() / v.len() as f32)
}

/// Gap below which a note glides from the previous note's correction (otherwise from 0).
const ADJACENT_SECONDS: f64 = 0.1;

pub fn curves(edit: &PitchEdit, track: &PitchTrack) -> Curves {
    let len = track.frames.len();
    let mut pitch = vec![0.0f32; len];
    let mut formant = vec![0.0f32; len];
    let hop = track.hop_seconds;
    let mut previous: Option<&EditNote> = None;
    for n in &edit.notes {
        let range: Vec<usize> = frames_in(track, n.start, n.end).collect();
        let (Some(&a), Some(&b)) = (range.first(), range.last()) else {
            previous = Some(n);
            continue;
        };
        let from = previous
            .filter(|p| n.start - p.end < ADJACENT_SECONDS)
            .map_or(0.0, EditNote::offset);
        for i in a..=b {
            let t = track.time(i) - n.start;
            let offset = if n.transition > 0.0 && (t as f32) < n.transition {
                from + (n.offset() - from) * (t as f32 / n.transition)
            } else {
                n.offset()
            };
            let shape = match (voiced(&track.frames[i]), drift_at(track, i, a, b + 1)) {
                (Some(m), Some(d)) => {
                    (n.drift - 1.0) * (d - n.centre) + (n.modulation - 1.0) * (m - d)
                }
                _ => 0.0,
            };
            pitch[i] = offset + shape;
            formant[i] = n.formant;
        }
        previous = Some(n);
    }
    let time = TimeMap::new(
        edit.notes
            .iter()
            .flat_map(|n| [(n.out_start, n.start), (n.out_end, n.end)])
            .chain([(0.0, 0.0), (len as f64 * hop, len as f64 * hop)])
            .collect(),
    );
    Curves {
        pitch,
        formant,
        time,
    }
}

/// The pitch after the edits, per frame of the output timeline (`None` when unvoiced).
pub fn edited_pitch(edit: &PitchEdit, track: &PitchTrack) -> Vec<Option<f32>> {
    let c = curves(edit, track);
    (0..track.frames.len())
        .map(|j| {
            let src = c.time.source(track.time(j)) / track.hop_seconds;
            let i = src.round() as usize;
            let m = voiced(track.frames.get(i)?)?;
            Some(m + c.pitch[i])
        })
        .collect()
}

/// Renders `audio` (the source on the song timeline) with the edits applied. Only the stretches
/// that change are processed, crossfaded into the untouched audio (AGENTS.md §7.1 step 4).
pub fn render(edit: &PitchEdit, track: &PitchTrack, audio: &[f32], sample_rate: u32) -> Vec<f32> {
    let c = curves(edit, track);
    let warped = stretch::warp(audio, sample_rate, &c.time);
    // The corrections, moved onto the output timeline.
    let hop = track.hop_seconds;
    let resample = |curve: &[f32]| -> Vec<f32> {
        (0..curve.len())
            .map(|j| {
                let i = (c.time.source(j as f64 * hop) / hop).round() as usize;
                curve.get(i).copied().unwrap_or(0.0)
            })
            .collect()
    };
    let pitch = resample(&c.pitch);
    let formant = resample(&c.formant);
    let mut out = warped.clone();
    let sr = f64::from(sample_rate);
    for (a, b) in changed_spans(&pitch, &formant, hop) {
        let start = ((a - RENDER_MARGIN_SECONDS).max(0.0) * sr) as usize;
        let end = (((b + RENDER_MARGIN_SECONDS) * sr) as usize).min(warped.len());
        if end <= start {
            continue;
        }
        let offset = start as f64 / sr / hop;
        let slice_curve = |curve: &[f32]| -> Vec<f32> {
            let first = offset.floor() as usize;
            curve.iter().skip(first).copied().collect()
        };
        // Shift the slice with curves re-based on its start (whole frames; the sub-frame
        // remainder is below the curve's own interpolation step).
        let shifted = shift::shift_curves(
            &warped[start..end],
            sample_rate,
            &slice_curve(&pitch),
            &slice_curve(&formant),
            hop,
        );
        let fade = ((RENDER_FADE_SECONDS * sr) as usize)
            .min((end - start) / 2)
            .max(1);
        let len = end - start;
        for (k, s) in shifted.iter().enumerate() {
            let w = if k < fade {
                k as f32 / fade as f32
            } else if k >= len - fade {
                (len - k) as f32 / fade as f32
            } else {
                1.0
            };
            out[start + k] = (1.0 - w) * warped[start + k] + w * s;
        }
    }
    out
}

/// Output-time stretches (seconds) where either curve is non-zero, merged when close.
fn changed_spans(pitch: &[f32], formant: &[f32], hop: f64) -> Vec<(f64, f64)> {
    let mut spans: Vec<(f64, f64)> = Vec::new();
    for i in 0..pitch.len() {
        if pitch[i].abs() < 1e-4 && formant[i].abs() < 1e-4 {
            continue;
        }
        let t = i as f64 * hop;
        match spans.last_mut() {
            Some(s) if t - s.1 <= 2.0 * RENDER_MARGIN_SECONDS => s.1 = t + hop,
            _ => spans.push((t, t + hop)),
        }
    }
    spans
}

/// An edit list seeded from auto-tune's settings (the editable correction curve, AGENTS.md §7
/// step 5): every note centred on the note auto-tune would pull it to, so the curve can then be
/// adjusted note by note in the editor.
pub fn from_tune(
    source: EditSource,
    track: &PitchTrack,
    key: Key,
    reference: &[melody::Note],
    settings: TuneSettings,
) -> PitchEdit {
    let settings = settings.clamped();
    let mut edit = analyze(source, track, key, settings.scale);
    let curve = crate::tune::correction_curve(track, reference, settings);
    for n in &mut edit.notes {
        let targets: Vec<f32> = frames_in(track, n.start, n.end)
            .filter_map(|i| curve[i].target)
            .collect();
        if let Some(t) = most_common(&targets) {
            n.pitch = n.centre + settings.mix * (t - n.centre);
        }
        n.formant = settings.formant_semitones;
    }
    edit
}

fn most_common(v: &[f32]) -> Option<f32> {
    let mut best: Option<(f32, usize)> = None;
    for &x in v {
        let count = v.iter().filter(|&&y| y == x).count();
        if best.is_none_or(|(_, c)| count > c) {
            best = Some((x, count));
        }
    }
    best.map(|(x, _)| x)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::key::Mode;
    use crate::melody::tests::synthetic_track;
    use crate::pitch;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    const C_MAJOR: Key = Key {
        tonic: 0,
        mode: Mode::Major,
    };
    const SR: u32 = 44_100;

    fn c_major() -> Scale {
        Scale::Key { key: C_MAJOR }
    }

    /// Two notes: 60.3 for 0.5 s (with vibrato), a gap, 64 for 0.5 s.
    fn two_notes() -> PitchTrack {
        synthetic_track(&[(Some(60.3), 0.5), (None, 0.2), (Some(64.0), 0.5)], 30.0)
    }

    fn edit() -> (PitchEdit, PitchTrack) {
        let track = two_notes();
        (
            analyze(EditSource::VocalStem, &track, C_MAJOR, c_major()),
            track,
        )
    }

    #[test]
    fn analysis_finds_notes_with_their_centres() {
        let (e, _) = edit();
        assert_eq!(e.notes.len(), 2);
        assert!(
            (e.notes[0].centre - 60.3).abs() < 0.15,
            "{}",
            e.notes[0].centre
        );
        assert!((e.notes[1].centre - 64.0).abs() < 0.15);
        assert!(e.notes.iter().all(|n| !n.edited()));
    }

    #[test]
    fn untouched_edit_list_asks_for_nothing() {
        let (e, track) = edit();
        let c = curves(&e, &track);
        assert!(c.pitch.iter().all(|v| *v == 0.0));
        assert!(c.formant.iter().all(|v| *v == 0.0));
        assert_eq!(c.time.changed(), None);
    }

    #[test]
    fn centre_correction_pulls_by_the_share_asked() {
        let (mut e, track) = edit();
        let id = e.notes[0].id;
        let centre = e.notes[0].centre;
        apply(
            &mut e,
            &EditOp::Correct {
                ids: vec![id],
                centre: Some(0.5),
                drift: None,
            },
            &track,
        );
        assert!((e.notes[0].pitch - (centre + 0.5 * (60.0 - centre))).abs() < 1e-5);
        // The curve moves every frame of the note by the offset (drift and vibrato kept).
        let c = curves(&e, &track);
        let i = (0.25 / track.hop_seconds) as usize;
        assert!((c.pitch[i] - (e.notes[0].pitch - centre)).abs() < 1e-5);
    }

    #[test]
    fn modulation_and_drift_scale_the_shape_around_the_centre() {
        let (mut e, track) = edit();
        let id = e.notes[0].id;
        apply(
            &mut e,
            &EditOp::SetModulation {
                ids: vec![id],
                modulation: 0.0,
            },
            &track,
        );
        apply(
            &mut e,
            &EditOp::Correct {
                ids: vec![id],
                centre: None,
                drift: Some(1.0),
            },
            &track,
        );
        // Flat vibrato and no drift: every voiced frame of the note lands on the centre.
        let edited = edited_pitch(&e, &track);
        let n = e.notes[0];
        for i in frames_in(&track, n.start + 0.1, n.end - 0.1) {
            let m = edited[i].expect("voiced");
            assert!(
                (m - n.centre).abs() < 0.02,
                "frame {i}: {m} vs {}",
                n.centre
            );
        }
        // Double vibrato: the spread around the centre doubles.
        apply(&mut e, &EditOp::Reset { ids: vec![id] }, &track);
        apply(
            &mut e,
            &EditOp::SetModulation {
                ids: vec![id],
                modulation: 2.0,
            },
            &track,
        );
        let spread = |v: &[Option<f32>]| {
            frames_in(&track, n.start + 0.1, n.end - 0.1)
                .filter_map(|i| v[i])
                .map(|m| (m - n.centre).abs())
                .fold(0.0f32, f32::max)
        };
        let sung: Vec<Option<f32>> = track.frames.iter().map(voiced).collect();
        let ratio = spread(&edited_pitch(&e, &track)) / spread(&sung);
        assert!((ratio - 2.0).abs() < 0.25, "{ratio}");
    }

    #[test]
    fn transitions_glide_from_the_previous_offset() {
        let track = synthetic_track(&[(Some(60.0), 0.5), (Some(62.0), 0.5)], 0.0);
        let mut e = analyze(EditSource::VocalStem, &track, C_MAJOR, c_major());
        assert_eq!(e.notes.len(), 2);
        let (a, b) = (e.notes[0].id, e.notes[1].id);
        apply(
            &mut e,
            &EditOp::MovePitch {
                ids: vec![a],
                semitones: 1.0,
                snap: Snap::Free,
            },
            &track,
        );
        apply(
            &mut e,
            &EditOp::SetTransition {
                ids: vec![b],
                seconds: 0.2,
            },
            &track,
        );
        let c = curves(&e, &track);
        let start = (e.notes[1].start / track.hop_seconds).ceil() as usize;
        let mid = start + (0.1 / track.hop_seconds) as usize;
        let after = start + (0.3 / track.hop_seconds) as usize;
        assert!((c.pitch[start] - 1.0).abs() < 0.1, "{}", c.pitch[start]);
        assert!((c.pitch[mid] - 0.5).abs() < 0.1, "{}", c.pitch[mid]);
        assert!(c.pitch[after].abs() < 1e-5);
    }

    #[test]
    fn pitch_moves_snap_as_asked() {
        let (mut e, track) = edit();
        let id = e.notes[0].id;
        let centre = e.notes[0].centre;
        apply(
            &mut e,
            &EditOp::MovePitch {
                ids: vec![id],
                semitones: 1.2,
                snap: Snap::Free,
            },
            &track,
        );
        assert!((e.notes[0].pitch - (centre + 1.2)).abs() < 1e-5);
        apply(
            &mut e,
            &EditOp::MovePitch {
                ids: vec![id],
                semitones: 0.0,
                snap: Snap::Semitone,
            },
            &track,
        );
        assert_eq!(e.notes[0].pitch, (centre + 1.2).round());
        // 61.x → C major snaps to 60 or 62, never the black key 61.
        apply(
            &mut e,
            &EditOp::MovePitch {
                ids: vec![id],
                semitones: 0.0,
                snap: Snap::Scale,
            },
            &track,
        );
        assert!(
            [60.0, 62.0].contains(&e.notes[0].pitch),
            "{}",
            e.notes[0].pitch
        );
    }

    #[test]
    fn time_edits_stay_between_neighbours_and_within_limits() {
        let (mut e, track) = edit();
        let (a, b) = (e.notes[0], e.notes[1]);
        // Moving the first note far right stops at the second note.
        apply(
            &mut e,
            &EditOp::MoveTime {
                id: a.id,
                out_start: 10.0,
            },
            &track,
        );
        assert!((e.notes[0].out_end - b.out_start).abs() < 1e-9);
        // A stretch is limited to 2× the sung length.
        apply(&mut e, &EditOp::Reset { ids: vec![a.id] }, &track);
        apply(
            &mut e,
            &EditOp::Stretch {
                id: b.id,
                out_start: b.start,
                out_end: b.start + 100.0,
            },
            &track,
        );
        assert!((e.notes[1].out_end - e.notes[1].out_start - 2.0 * (b.end - b.start)).abs() < 1e-9);
        assert!(curves(&e, &track).time.changed().is_some());
    }

    #[test]
    fn split_and_merge_round_trip() {
        let (mut e, track) = edit();
        let n = e.notes[0];
        let at = (n.start + n.end) / 2.0;
        apply(&mut e, &EditOp::Split { id: n.id, at }, &track);
        assert_eq!(e.notes.len(), 3);
        assert_eq!(e.notes[0].end, at);
        assert_eq!(e.notes[1].start, at);
        assert!(
            e.notes[1].id
                > e.notes
                    .iter()
                    .map(|n| n.id)
                    .filter(|&i| i != e.notes[1].id)
                    .max()
                    .unwrap_or(0)
                    - 1
        );
        apply(&mut e, &EditOp::Merge { id: n.id }, &track);
        assert_eq!(e.notes.len(), 2);
        assert_eq!((e.notes[0].start, e.notes[0].end), (n.start, n.end));
        // Splits too close to an edge are refused.
        apply(
            &mut e,
            &EditOp::Split {
                id: n.id,
                at: n.start + 0.01,
            },
            &track,
        );
        assert_eq!(e.notes.len(), 2);
    }

    /// A sung-like tone: harmonics of `hz` with a vowel-ish rolloff.
    fn voice(hz: f32, seconds: f64) -> Vec<f32> {
        let n = (seconds * f64::from(SR)) as usize;
        (0..n)
            .map(|i| {
                let t = i as f32 / SR as f32;
                (1..=8)
                    .map(|k| {
                        0.3 / k as f32 * (2.0 * std::f32::consts::PI * hz * k as f32 * t).sin()
                    })
                    .sum()
            })
            .collect()
    }

    #[test]
    fn rendering_moves_a_note_and_leaves_the_rest_alone() {
        // 1 s at A3, 0.5 s silence, 1 s at C4, 0.5 s silence.
        let mut audio = voice(220.0, 1.0);
        audio.extend(vec![0.0; SR as usize / 2]);
        audio.extend(voice(261.63, 1.0));
        audio.extend(vec![0.0; SR as usize / 2]);
        let track = pitch::track(&audio, SR);
        let mut e = analyze(EditSource::VocalStem, &track, C_MAJOR, Scale::Chromatic);
        assert_eq!(e.notes.len(), 2, "{:?}", e.notes);
        let second = e.notes[1].id;
        apply(
            &mut e,
            &EditOp::MovePitch {
                ids: vec![second],
                semitones: 2.0,
                snap: Snap::Free,
            },
            &track,
        );
        let out = render(&e, &track, &audio, SR);
        assert_eq!(out.len(), audio.len());

        // The moved note lands within 5 cents of +2 st.
        let after = pitch::track(&out, SR);
        let Some(PitchFrame::Voiced { midi }) = after.frame_at(2.0).copied() else {
            panic!("unvoiced");
        };
        let expected = crate::note::hz_to_midi(261.63) + 2.0;
        assert!(
            (midi - expected).abs() * 100.0 < 5.0,
            "{midi} vs {expected}"
        );

        // The untouched first note is bit-identical.
        let untouched = (1.2 * f64::from(SR)) as usize;
        assert_eq!(&out[..untouched], &audio[..untouched]);
    }

    #[test]
    fn seeding_from_auto_tune_targets_the_scale() {
        let track = two_notes();
        let settings = TuneSettings {
            scale: c_major(),
            mix: 1.0,
            ..TuneSettings::default()
        };
        let e = from_tune(EditSource::VocalStem, &track, C_MAJOR, &[], settings);
        assert_eq!(e.notes[0].pitch, 60.0);
        assert_eq!(e.notes[1].pitch, 64.0);
    }
}
