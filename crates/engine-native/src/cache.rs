//! Stem cache: one directory per `sha256(source file) + model id`, holding the stems as WAV
//! and the analyzed `project.json` (AGENTS.md §4.2). A cached song is also its saved project.

use crate::decode;
use anyhow::{Context, Result};
use engine_core::project::{Project, SongAudio, TrackSource};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Model id used for songs imported without a separation model.
pub const UNSEPARATED: &str = "unseparated";

pub enum Lookup {
    Hit {
        project: Box<Project>,
        audio: SongAudio,
    },
    Miss,
}

pub struct StemCache {
    root: PathBuf,
}

impl StemCache {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub fn key(source: &Path, model_id: &str) -> Result<String> {
        let bytes =
            std::fs::read(source).with_context(|| format!("reading {}", source.display()))?;
        let digest = Sha256::digest(&bytes);
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        Ok(format!("{hex}-{model_id}"))
    }

    fn dir(&self, key: &str) -> PathBuf {
        self.root.join(key)
    }

    pub fn lookup(&self, key: &str) -> Result<Lookup> {
        let dir = self.dir(key);
        let manifest = dir.join("project.json");
        if !manifest.exists() {
            return Ok(Lookup::Miss);
        }
        let project: Project = serde_json::from_str(&std::fs::read_to_string(&manifest)?)?;
        let load = |source: &TrackSource| decode::decode(&dir.join(file_name(source)));
        let audio = match project.tracks.as_slice() {
            [only] if only.source == TrackSource::Mix => SongAudio::Unseparated {
                mix: load(&only.source)?,
            },
            tracks => SongAudio::Separated {
                stems: tracks
                    .iter()
                    .map(|t| match t.source {
                        TrackSource::Stem { stem } => Ok((stem, load(&t.source)?)),
                        TrackSource::Mix => anyhow::bail!("mix track inside a separated project"),
                    })
                    .collect::<Result<_>>()?,
            },
        };
        Ok(Lookup::Hit {
            project: Box::new(project),
            audio,
        })
    }

    /// Writes stems first and the manifest last, so a partial entry is never a hit.
    pub fn store(&self, key: &str, project: &Project, audio: &SongAudio) -> Result<()> {
        let dir = self.dir(key);
        std::fs::create_dir_all(&dir)?;
        match audio {
            SongAudio::Unseparated { mix } => {
                decode::write_wav(&dir.join(file_name(&TrackSource::Mix)), mix)?
            }
            SongAudio::Separated { stems } => {
                for (stem, a) in stems {
                    decode::write_wav(&dir.join(file_name(&TrackSource::Stem { stem: *stem })), a)?;
                }
            }
        }
        std::fs::write(dir.join("project.json"), serde_json::to_string(project)?)?;
        Ok(())
    }

    /// Persists edited mix settings of a cached project.
    pub fn save_project(&self, key: &str, project: &Project) -> Result<()> {
        std::fs::write(
            self.dir(key).join("project.json"),
            serde_json::to_string(project)?,
        )?;
        Ok(())
    }
}

fn file_name(source: &TrackSource) -> String {
    match source {
        TrackSource::Mix => "mix.wav".to_string(),
        TrackSource::Stem { stem } => format!("{stem:?}.wav").to_lowercase(),
    }
}
