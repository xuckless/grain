//! The `.grain` project archive (AGENTS.md §12), the desktop side. The web host streams the same
//! layout one channel at a time from TypeScript (apps/web/src/store.ts) to stay inside the main
//! thread's frame budget; the round-trip test below pins the layout both follow:
//!
//! ```text
//! 8 bytes  magic "GRAINv1\n"
//! u32 LE   header length H
//! H bytes  UTF-8 JSON { "project": Project }
//! then 16-bit little-endian PCM channels, each `project.frames` samples: every track's left
//! then right channel, then every take's timeline audio (mono, raw: chains re-render on open).
//! ```

use crate::audio::StereoAudio;
use crate::project::Project;
use serde::{Deserialize, Serialize};

const MAGIC: &[u8; 8] = b"GRAINv1\n";

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("this isn't a Grain project file")]
    NotArchive,
    #[error("the project file is incomplete")]
    Incomplete,
    #[error("the project file's header is damaged: {0}")]
    Header(String),
}

#[derive(Serialize, Deserialize)]
struct Header {
    project: Project,
}

/// A whole archive: tracks in project order, takes in project order.
pub struct Archive {
    pub project: Project,
    pub tracks: Vec<StereoAudio>,
    pub takes: Vec<Vec<f32>>,
}

fn pcm16(out: &mut Vec<u8>, x: &[f32], frames: usize) {
    for i in 0..frames {
        let v = x.get(i).copied().unwrap_or(0.0);
        out.extend(((v * 32767.0).round().clamp(-32768.0, 32767.0) as i16).to_le_bytes());
    }
}

pub fn encode(project: &Project, tracks: &[&StereoAudio], takes: &[Vec<f32>]) -> Vec<u8> {
    let json = serde_json::to_vec(&Header {
        project: project.clone(),
    })
    .expect("a project serializes");
    let frames = project.frames as usize;
    let mut out =
        Vec::with_capacity(12 + json.len() + frames * 2 * (2 * tracks.len() + takes.len()));
    out.extend(MAGIC);
    out.extend((json.len() as u32).to_le_bytes());
    out.extend(&json);
    for t in tracks {
        pcm16(&mut out, &t.left, frames);
        pcm16(&mut out, &t.right, frames);
    }
    for t in takes {
        pcm16(&mut out, t, frames);
    }
    out
}

pub fn decode(bytes: &[u8]) -> Result<Archive, ArchiveError> {
    if bytes.len() < 12 || &bytes[..8] != MAGIC {
        return Err(ArchiveError::NotArchive);
    }
    let h = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]) as usize;
    let json = bytes.get(12..12 + h).ok_or(ArchiveError::Incomplete)?;
    let Header { project } =
        serde_json::from_slice(json).map_err(|e| ArchiveError::Header(e.to_string()))?;
    let frames = project.frames as usize;
    let channels = 2 * project.tracks.len() + project.takes.len();
    let audio = &bytes[12 + h..];
    if audio.len() != frames * 2 * channels {
        return Err(ArchiveError::Incomplete);
    }
    let mut chunks = audio.chunks_exact(frames * 2).map(|c| {
        c.chunks_exact(2)
            .map(|b| f32::from(i16::from_le_bytes([b[0], b[1]])) / 32767.0)
            .collect::<Vec<f32>>()
    });
    let tracks = (0..project.tracks.len())
        .map(|_| StereoAudio {
            sample_rate: project.sample_rate,
            left: chunks.next().unwrap_or_default(),
            right: chunks.next().unwrap_or_default(),
        })
        .collect();
    let takes = chunks.collect();
    Ok(Archive {
        project,
        tracks,
        takes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::{self, SongAudio, SongSource, TrackedPitch};
    use crate::take::{add_take, TakeSource};
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn an_archive_round_trips_tracks_and_takes() {
        let sr = 8_000;
        let tone: Vec<f32> = (0..sr as usize)
            .map(|i| 0.5 * (i as f32 * 0.05).sin())
            .collect();
        let mix = StereoAudio {
            sample_rate: sr,
            left: tone.clone(),
            right: tone.iter().map(|v| -v).collect(),
        };
        let mut p = project::build(
            SongSource::File { name: "s".into() },
            &SongAudio::Unseparated { mix: mix.clone() },
            TrackedPitch::yin(&tone, sr),
        );
        add_take(
            &mut p,
            TakeSource::Recorded { latency_ms: 0.0 },
            &tone,
            TrackedPitch::yin(&tone, sr),
        );
        let bytes = encode(&p, &[&mix], std::slice::from_ref(&tone));
        assert_eq!(&bytes[..8], MAGIC);
        let back = decode(&bytes).unwrap();
        assert_eq!(back.project.takes.len(), 1);
        assert!((back.tracks[0].right[100] - mix.right[100]).abs() < 1e-4);
        assert!((back.takes[0][100] - tone[100]).abs() < 1e-4);
        assert!(matches!(
            decode(&bytes[..bytes.len() - 2]),
            Err(ArchiveError::Incomplete)
        ));
        assert!(matches!(decode(b"nope"), Err(ArchiveError::NotArchive)));
    }
}
