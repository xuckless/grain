//! Song import pipeline: decode → separate → analyze → cache (AGENTS.md §4).

use crate::cache::{Lookup, StemCache, UNSEPARATED};
use crate::decode;
use crate::separate::Demucs;
use anyhow::Result;
use engine_core::project::{self, Project, SongAudio, SongSource};
use engine_core::WORKING_SAMPLE_RATE;
use serde::Serialize;
use std::path::Path;

/// Progress of one import, reported as it advances.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind")]
pub enum ImportStage {
    Decoding,
    Separating { segment: usize, of: usize },
    Analyzing,
    Caching,
}

pub enum Separation<'a> {
    Model(&'a mut Demucs),
    /// No model installed: the song stays one mix track.
    Disabled,
}

pub struct Imported {
    pub cache_key: String,
    pub project: Project,
    pub audio: SongAudio,
}

pub fn import_song(
    path: &Path,
    song: SongSource,
    separation: Separation,
    cache: &StemCache,
    mut progress: impl FnMut(ImportStage),
) -> Result<Imported> {
    let model_id = match &separation {
        Separation::Model(m) => m.id().to_string(),
        Separation::Disabled => UNSEPARATED.to_string(),
    };
    let cache_key = StemCache::key(path, &model_id)?;
    if let Lookup::Hit { project, audio } = cache.lookup(&cache_key)? {
        return Ok(Imported {
            cache_key,
            project: *project,
            audio,
        });
    }
    progress(ImportStage::Decoding);
    let mix = decode::load(path, WORKING_SAMPLE_RATE)?;
    let audio = match separation {
        Separation::Model(model) => SongAudio::Separated {
            stems: model.separate(&mix, |segment, of| {
                progress(ImportStage::Separating { segment, of })
            })?,
        },
        Separation::Disabled => SongAudio::Unseparated { mix },
    };
    progress(ImportStage::Analyzing);
    let project = project::build(song, &audio);
    progress(ImportStage::Caching);
    cache.store(&cache_key, &project, &audio)?;
    Ok(Imported {
        cache_key,
        project,
        audio,
    })
}
