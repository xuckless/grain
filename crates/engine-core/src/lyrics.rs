//! Song lyrics for karaoke (AGENTS.md §6.1): LRC parsing, and syllable timing against the
//! reference notes (§6.2) in the simplest robust way: each line's syllables (CJK characters,
//! or space-separated words) are spread over the note onsets inside the line, UltraStar style.
//! ML forced alignment (CTC / Whisper word timestamps) is a later upgrade.
//!
//! Lyric text is fetched at runtime by the UI (LRCLIB) and only ever stored with the user's own
//! project; nothing here bundles or redistributes it.

use crate::melody::Note;
use crate::project::Project;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What the song was looked up by (editable by the user).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LyricsQuery {
    pub artist: String,
    pub title: String,
}

/// One sung unit of a line: a CJK character (with attached small kana and punctuation) or a
/// word (with its trailing space), so the units of a line concatenate back to the line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Syllable {
    pub start: f64,
    pub end: f64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LyricLine {
    /// Start time as written in the LRC (before the user's offset).
    pub lrc_start: f64,
    /// Song time the line starts and ends (offset applied; ends where the next line starts).
    pub start: f64,
    pub end: f64,
    pub text: String,
    /// Empty for an empty (instrumental break) line.
    pub syllables: Vec<Syllable>,
}

/// The lyrics stored with a project.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum ProjectLyrics {
    /// Never looked up (lyrics are fetched only when the song enters karaoke).
    #[default]
    NotFetched,
    NotFound {
        query: LyricsQuery,
    },
    /// Unsynced text only: shown, not timed.
    Plain {
        query: LyricsQuery,
        lines: Vec<String>,
    },
    Synced {
        query: LyricsQuery,
        /// The user's global nudge; positive shows the lyrics later.
        offset_ms: f32,
        lines: Vec<LyricLine>,
    },
}

/// A line as the user edits it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct LineEdit {
    pub lrc_start: f64,
    pub text: String,
}

/// A change to a project's lyrics.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(tag = "kind")]
#[ts(export)]
pub enum LyricsUpdate {
    /// An LRCLIB result: synced LRC and/or plain text.
    Fetched {
        query: LyricsQuery,
        synced: Option<String>,
        plain: Option<String>,
    },
    NotFound {
        query: LyricsQuery,
    },
    /// Offset nudge or line edits of synced lyrics.
    Edited {
        offset_ms: f32,
        lines: Vec<LineEdit>,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum LyricsError {
    #[error("only synced lyrics can be edited")]
    NotSynced,
}

/// Applies `update` to the project's lyrics and returns the new lyrics.
pub fn apply(project: &mut Project, update: LyricsUpdate) -> Result<ProjectLyrics, LyricsError> {
    let song_end = f64::from(project.frames) / f64::from(project.sample_rate);
    let notes = &project.analysis.reference_notes;
    let lyrics = match update {
        LyricsUpdate::Fetched {
            query,
            synced,
            plain,
        } => {
            let parsed = synced.as_deref().map(parse_lrc).unwrap_or_default();
            let plain_lines: Vec<String> = plain
                .as_deref()
                .map(|p| p.lines().map(|l| l.trim().to_string()).collect())
                .unwrap_or_default();
            if parsed.iter().any(|l| !l.text.is_empty()) {
                ProjectLyrics::Synced {
                    query,
                    offset_ms: 0.0,
                    lines: time_lines(&parsed, 0.0, notes, song_end),
                }
            } else if plain_lines.iter().any(|l| !l.is_empty()) {
                ProjectLyrics::Plain {
                    query,
                    lines: plain_lines,
                }
            } else {
                ProjectLyrics::NotFound { query }
            }
        }
        LyricsUpdate::NotFound { query } => ProjectLyrics::NotFound { query },
        LyricsUpdate::Edited { offset_ms, lines } => {
            let ProjectLyrics::Synced { query, .. } = &project.lyrics else {
                return Err(LyricsError::NotSynced);
            };
            let mut lines = lines;
            lines.sort_by(|a, b| a.lrc_start.total_cmp(&b.lrc_start));
            ProjectLyrics::Synced {
                query: query.clone(),
                offset_ms,
                lines: time_lines(&lines, offset_ms, notes, song_end),
            }
        }
    };
    project.lyrics = lyrics.clone();
    Ok(lyrics)
}

/// Lines of an LRC file in time order: `[mm:ss.xx]` tags (several per line allowed), the
/// `[offset:±ms]` tag applied, metadata tags and enhanced-LRC `<mm:ss.xx>` word tags dropped.
pub fn parse_lrc(text: &str) -> Vec<LineEdit> {
    let offset_ms = text
        .lines()
        .find_map(|l| {
            let tag = l.trim().strip_prefix("[offset:")?.strip_suffix(']')?;
            tag.trim().parse::<f64>().ok()
        })
        .unwrap_or(0.0);
    let mut out = Vec::new();
    for line in text.lines() {
        let mut rest = line.trim();
        let mut times = Vec::new();
        while let Some(after) = rest.strip_prefix('[') {
            let Some(close) = after.find(']') else { break };
            match parse_time(&after[..close]) {
                Some(t) => times.push(t),
                None if times.is_empty() => break, // a metadata tag: not a lyric line
                None => {}
            }
            rest = &after[close + 1..];
        }
        if times.is_empty() {
            continue;
        }
        let text = strip_word_tags(rest).trim().to_string();
        for t in times {
            out.push(LineEdit {
                // LRC: a positive offset makes the lyrics appear sooner.
                lrc_start: (t - offset_ms / 1000.0).max(0.0),
                text: text.clone(),
            });
        }
    }
    out.sort_by(|a, b| a.lrc_start.total_cmp(&b.lrc_start));
    out
}

/// `mm:ss`, `mm:ss.x`, `mm:ss.xx` or `mm:ss.xxx` in seconds.
fn parse_time(tag: &str) -> Option<f64> {
    let (m, s) = tag.split_once(':')?;
    let m: u32 = m.trim().parse().ok()?;
    let s: f64 = s.trim().parse().ok()?;
    (0.0..60.0).contains(&s).then_some(f64::from(m) * 60.0 + s)
}

fn strip_word_tags(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('<') {
        out.push_str(&rest[..open]);
        match rest[open..].find('>') {
            Some(close) if parse_time(&rest[open + 1..open + close]).is_some() => {
                rest = &rest[open + close + 1..];
            }
            _ => {
                out.push('<');
                rest = &rest[open + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Times every line: effective start = LRC start + offset; a line ends where the next begins.
fn time_lines(lines: &[LineEdit], offset_ms: f32, notes: &[Note], song_end: f64) -> Vec<LyricLine> {
    let shift = f64::from(offset_ms) / 1000.0;
    let starts: Vec<f64> = lines
        .iter()
        .map(|l| (l.lrc_start + shift).max(0.0))
        .collect();
    lines
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let start = starts[i];
            let end = starts.get(i + 1).copied().unwrap_or(song_end).max(start);
            LyricLine {
                lrc_start: l.lrc_start,
                start,
                end,
                text: l.text.clone(),
                syllables: time_syllables(&split_units(&l.text), start, end, notes),
            }
        })
        .collect()
}

fn is_cjk(c: char) -> bool {
    matches!(c,
        '\u{3040}'..='\u{30FF}' // hiragana, katakana
        | '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}' | '\u{F900}'..='\u{FAFF}' // han
        | '\u{AC00}'..='\u{D7AF}' // hangul syllables
        | '\u{FF66}'..='\u{FF9F}') // half-width katakana
}

/// Characters sung together with the one before: small kana, the long-vowel mark, and closing
/// punctuation.
fn attaches_back(c: char) -> bool {
    "ぁぃぅぇぉっゃゅょゎァィゥェォッャュョヮヵヶーゝゞヽヾ、。，．！？!?,.」』）)…〜~・"
        .contains(c)
}

/// Opening punctuation, sung with the character after it.
fn attaches_forward(c: char) -> bool {
    "「『（(\"'“‘".contains(c)
}

/// The sung units of a line (see [`Syllable`]); they concatenate back to `text`.
pub fn split_units(text: &str) -> Vec<String> {
    let mut units: Vec<String> = Vec::new();
    let mut pending = String::new(); // opening punctuation waiting for its character
    let mut word = String::new(); // a non-CJK word being collected
    let flush = |word: &mut String, units: &mut Vec<String>| {
        if !word.is_empty() {
            units.push(std::mem::take(word));
        }
    };
    for c in text.chars() {
        if c.is_whitespace() {
            flush(&mut word, &mut units);
            match units.last_mut() {
                Some(last) if pending.is_empty() => last.push(c),
                _ => pending.push(c),
            }
        } else if attaches_back(c) && word.is_empty() && pending.is_empty() && !units.is_empty() {
            units.last_mut().expect("checked").push(c);
        } else if attaches_forward(c) && word.is_empty() {
            pending.push(c);
        } else if is_cjk(c) {
            flush(&mut word, &mut units);
            units.push(std::mem::take(&mut pending) + &c.to_string());
        } else {
            if word.is_empty() {
                word = std::mem::take(&mut pending);
            }
            word.push(c);
        }
    }
    flush(&mut word, &mut units);
    if !pending.is_empty() {
        match units.last_mut() {
            Some(last) => last.push_str(&pending),
            None => units.push(pending),
        }
    }
    units
}

/// Spreads `units` over the reference notes starting in `[start, end)`. With at least as many
/// notes as units, each unit takes a run of notes (melisma); with fewer notes, each note's time
/// is split evenly between its units. Without notes, the units share the line evenly.
pub fn time_syllables(units: &[String], start: f64, end: f64, notes: &[Note]) -> Vec<Syllable> {
    let u = units.len();
    if u == 0 {
        return Vec::new();
    }
    let inside: Vec<&Note> = notes
        .iter()
        .filter(|n| n.start >= start && n.start < end)
        .collect();
    let n = inside.len();
    let syllable = |k: usize, s: f64, e: f64| Syllable {
        start: s,
        end: e.max(s),
        text: units[k].clone(),
    };
    if n == 0 {
        let step = (end - start) / u as f64;
        return (0..u)
            .map(|k| syllable(k, start + k as f64 * step, start + (k + 1) as f64 * step))
            .collect();
    }
    if u <= n {
        (0..u)
            .map(|k| {
                let (first, last) = (k * n / u, ((k + 1) * n / u).max(k * n / u + 1) - 1);
                syllable(k, inside[first].start, inside[last].end.min(end))
            })
            .collect()
    } else {
        let mut out = Vec::with_capacity(u);
        for (j, note) in inside.iter().enumerate() {
            let (first, last) = (j * u / n, (j + 1) * u / n);
            let span = (note.end.min(end) - note.start) / (last - first) as f64;
            for k in first..last {
                let s = note.start + (k - first) as f64 * span;
                out.push(syllable(k, s, s + span));
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    fn note(start: f64, end: f64) -> Note {
        Note {
            start,
            end,
            midi: 60,
        }
    }

    fn texts(units: &[String]) -> Vec<&str> {
        units.iter().map(String::as_str).collect()
    }

    #[test]
    fn parses_lrc_with_offsets_repeats_and_metadata() {
        let lrc = "[ar:Someone]\n[ti:Song]\n[offset:+500]\n[00:12.05]first line\n\
                   [00:20.00][01:05.5]chorus <00:20.50>with <00:21.00>tags\n[00:30]\n\
                   not a lyric\n[1:02.123] last";
        let lines = parse_lrc(lrc);
        let got: Vec<(f64, &str)> = lines
            .iter()
            .map(|l| ((l.lrc_start * 1000.0).round() / 1000.0, l.text.as_str()))
            .collect();
        assert_eq!(
            got,
            vec![
                (11.55, "first line"),
                (19.5, "chorus with tags"),
                (29.5, ""),
                (61.623, "last"),
                (65.0, "chorus with tags"),
            ]
        );
    }

    #[test]
    fn splits_cjk_by_character_and_latin_by_word() {
        assert_eq!(
            texts(&split_units("ちょっと冷たいけど")),
            vec!["ちょっ", "と", "冷", "た", "い", "け", "ど"]
        );
        assert_eq!(
            texts(&split_units("「愛するための」代償なら")),
            vec!["「愛", "す", "る", "た", "め", "の」", "代", "償", "な", "ら"]
        );
        assert_eq!(
            texts(&split_units("ねえ ダーリン")),
            vec!["ね", "え ", "ダー", "リ", "ン"]
        );
        assert_eq!(
            texts(&split_units("Oh 星屑もない")),
            vec!["Oh ", "星", "屑", "も", "な", "い"]
        );
        assert_eq!(
            texts(&split_units("Hello, my (dear) world!")),
            vec!["Hello, ", "my ", "(dear) ", "world!"]
        );
        for line in [
            "ふたりぼっちの世界でずっと",
            "Let it be, let it be",
            " 「x」 ",
        ] {
            assert_eq!(split_units(line).concat(), line);
        }
        assert!(split_units("").is_empty());
    }

    #[test]
    fn spreads_syllables_over_notes() {
        let units: Vec<String> = ["a", "b"].map(String::from).to_vec();
        // Fewer units than notes: each unit takes a run of notes.
        let notes = [
            note(1.0, 1.5),
            note(1.6, 2.0),
            note(2.1, 2.5),
            note(9.0, 9.5),
        ];
        let s = time_syllables(&units, 1.0, 3.0, &notes);
        assert_eq!(
            s.iter().map(|x| (x.start, x.end)).collect::<Vec<_>>(),
            vec![(1.0, 1.5), (1.6, 2.5)]
        );
        // More units than notes: notes are split evenly.
        let units: Vec<String> = ["a", "b", "c"].map(String::from).to_vec();
        let s = time_syllables(&units, 0.0, 3.0, &[note(1.0, 1.6)]);
        let spans: Vec<(f64, f64)> = s.iter().map(|x| (x.start, x.end)).collect();
        for (got, want) in spans.iter().zip([(1.0, 1.2), (1.2, 1.4), (1.4, 1.6)]) {
            assert!(
                (got.0 - want.0).abs() < 1e-9 && (got.1 - want.1).abs() < 1e-9,
                "{spans:?}"
            );
        }
        // No notes: the line is shared evenly.
        let s = time_syllables(&units, 0.0, 3.0, &[]);
        assert_eq!(
            s.iter().map(|x| x.start).collect::<Vec<_>>(),
            vec![0.0, 1.0, 2.0]
        );
        // Syllables never run backwards.
        assert!(s.windows(2).all(|w| w[0].end <= w[1].start + 1e-9));
    }

    #[test]
    fn lines_end_where_the_next_begins_and_offset_shifts_them() {
        let lines = parse_lrc("[00:01.00]ab\n[00:03.00]かな");
        let timed = time_lines(&lines, 500.0, &[], 10.0);
        assert_eq!((timed[0].start, timed[0].end), (1.5, 3.5));
        assert_eq!((timed[1].start, timed[1].end), (3.5, 10.0));
        assert_eq!(timed[0].lrc_start, 1.0);
        assert_eq!(timed[1].syllables.len(), 2);
    }
}
