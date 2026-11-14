//! Song import pipeline: decode → separate → analyze → cache (AGENTS.md §4).

use crate::cache::{Lookup, StemCache, UNSEPARATED};
use crate::decode;
use crate::pitch::Pitch;
use crate::separate::Separators;
use anyhow::Result;
use engine_core::project::{self, ImportStage, Project, SeparationQuality, SongAudio, SongSource};
use engine_core::WORKING_SAMPLE_RATE;
use std::path::Path;

pub enum Separation<'a> {
    Model {
        separators: &'a mut Separators,
        quality: SeparationQuality,
    },
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
    pitch: &mut Pitch,
    cache: &StemCache,
    mut progress: impl FnMut(ImportStage),
) -> Result<Imported> {
    let model_id = match &separation {
        Separation::Model {
            separators,
            quality,
        } => separators.id(*quality)?,
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
        Separation::Model {
            separators,
            quality,
        } => SongAudio::Separated {
            stems: separators.separate(quality, &mix, |segment, of| {
                progress(ImportStage::Separating {
                    segment: segment as u32,
                    of: of as u32,
                })
            })?,
        },
        Separation::Disabled => SongAudio::Unseparated { mix },
    };
    progress(ImportStage::Analyzing);
    let reference = pitch.track(
        pitch.preferred(),
        &project::reference_signal(&audio),
        WORKING_SAMPLE_RATE,
    )?;
    let project = project::build(song, &audio, reference);
    progress(ImportStage::Caching);
    cache.store(&cache_key, &project, &audio)?;
    Ok(Imported {
        cache_key,
        project,
        audio,
    })
}
