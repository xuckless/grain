//! Stem cache: one directory per `sha256(source file) + model id`, holding the stems as WAV,
//! the takes as `take-<id>.wav` (mono, on the song timeline) and the analyzed `project.json`
//! (AGENTS.md §4.2). A cached song is also its saved project.

use crate::decode;
use anyhow::{Context, Result};
use engine_core::audio::StereoAudio;
use engine_core::project::{self, Project, ProjectSummary, SongAudio, TrackSource};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Model id used for songs imported without a separation model.
pub const UNSEPARATED: &str = "unseparated";

pub enum Lookup {
    Hit {
        project: Box<Project>,
        audio: SongAudio,
        /// Timeline audio of each of `project.takes`, in order.
        takes: Vec<Vec<f32>>,
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
        let mut project: Project = serde_json::from_str(&std::fs::read_to_string(&manifest)?)?;
        let tracks = project
            .tracks
            .iter()
            .map(|t| decode::decode(&dir.join(file_name(&t.source))))
            .collect::<Result<_>>()?;
        let audio = project::restore_audio(&mut project, tracks)?;
        let takes = project
            .takes
            .iter()
            .map(|t| Ok(decode::decode(&dir.join(take_file(t.id)))?.left))
            .collect::<Result<_>>()?;
        Ok(Lookup::Hit {
            project: Box::new(project),
            audio,
            takes,
        })
    }

    /// Writes a take's timeline audio (mono, project rate). The take is part of the saved
    /// project once `save_project` writes a manifest that lists it.
    pub fn store_take(&self, key: &str, id: u32, timeline: &[f32], sample_rate: u32) -> Result<()> {
        let dir = self.dir(key);
        std::fs::create_dir_all(&dir)?;
        decode::write_wav(
            &dir.join(take_file(id)),
            &StereoAudio {
                sample_rate,
                left: timeline.to_vec(),
                right: timeline.to_vec(),
            },
        )
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

    /// Every complete cache entry, most recently saved first. Unreadable entries are skipped.
    pub fn list(&self) -> Result<Vec<ProjectSummary>> {
        let entries = match std::fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e.into()),
        };
        let mut list: Vec<ProjectSummary> = entries
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let manifest = entry.path().join("project.json");
                let saved = std::fs::metadata(&manifest).ok()?.modified().ok()?;
                let project: Project =
                    serde_json::from_str(&std::fs::read_to_string(&manifest).ok()?).ok()?;
                let saved_ms = saved
                    .duration_since(std::time::UNIX_EPOCH)
                    .ok()?
                    .as_millis() as f64;
                let key = entry.file_name().to_string_lossy().into_owned();
                Some(ProjectSummary::new(key, &project, saved_ms))
            })
            .collect();
        list.sort_by(|a, b| b.saved_ms.total_cmp(&a.saved_ms));
        Ok(list)
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

fn take_file(id: u32) -> String {
    format!("take-{id}.wav")
}

fn file_name(source: &TrackSource) -> String {
    match source {
        TrackSource::Mix => "mix.wav".to_string(),
        TrackSource::Stem { stem } => format!("{stem:?}.wav").to_lowercase(),
    }
}
