//! Export on desktop (AGENTS.md §8.8): the mixdown (tracks at their mix levels, takes through
//! their processing chains) as WAV-24, or MP3 / M4A encoded by the bundled ffmpeg; and the
//! `.grain` project archive, written and opened.

use crate::{chain, start_session, with_loaded, Engine};
use anyhow::Context;
use engine_core::archive;
use engine_core::export::{self, ExportFormat, ExportRequest, TrackRef};
use engine_core::process::ChainCache;
use engine_core::project::{self, Project, SongAudio};
use engine_core::take;
use engine_native::cache::{Lookup, StemCache};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::ipc::Channel;
use tauri::AppHandle;

/// Progress pushed while exporting (the UI's Rendering and Encoding states).
#[derive(Serialize, Clone)]
#[serde(tag = "kind")]
pub enum ExportProgress {
    Rendering { fraction: f32 },
    Encoding,
}

fn exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_default()
}

/// The ffmpeg MP3 and M4A are encoded with, if any.
pub fn ffmpeg() -> Option<PathBuf> {
    ingest::find_tool(&exe_dir(), "ffmpeg")
}

/// Formats this machine can write, with the reason for any it can't (`null`: available).
#[tauri::command(async)]
pub fn export_formats() -> Vec<(ExportFormat, Option<String>)> {
    let encoder = ffmpeg()
        .is_none()
        .then(|| "ffmpeg wasn't found, so only WAV can be written.".to_string());
    vec![
        (ExportFormat::Wav24, None),
        (ExportFormat::Mp3, encoder.clone()),
        (ExportFormat::M4a, encoder),
    ]
}

/// The loaded song, its stored audio, and each take's audio (through its chain when it's
/// exported, else empty).
/// What a mixdown reads: the project, the song's audio and each take's audio.
type Sources = (Project, SongAudio, Vec<Arc<[f32]>>);

fn sources(engine: &Engine, include: &[u32]) -> anyhow::Result<Sources> {
    let (key, project) = with_loaded(engine, |l| Ok((l.cache_key.clone(), l.project.clone())))?;
    let Lookup::Hit { audio, .. } = engine.cache.lookup(&key)? else {
        anyhow::bail!("this project is no longer on disk");
    };
    let mut takes = Vec::with_capacity(project.takes.len());
    for t in &project.takes {
        let index = take::player_index(&project, t.id).expect("listed");
        takes.push(if include.contains(&index) {
            let raw: Arc<[f32]> = engine.cache.take_audio(&key, t.id)?.into();
            chain::render(engine, &project, t.id, &raw, &mut ChainCache::default())?
        } else {
            Arc::from(Vec::new())
        });
    }
    Ok((project, audio, takes))
}

fn encode(path: &Path, format: ExportFormat, wav: &[u8]) -> anyhow::Result<()> {
    if format == ExportFormat::Wav24 {
        return Ok(std::fs::write(path, wav)?);
    }
    let ffmpeg = ffmpeg().context("ffmpeg wasn't found")?;
    let tmp = std::env::temp_dir().join(format!("grain-export-{}.wav", std::process::id()));
    std::fs::write(&tmp, wav)?;
    let codec: &[&str] = match format {
        ExportFormat::Mp3 => &["-c:a", "libmp3lame", "-b:a", "320k"],
        ExportFormat::M4a => &["-c:a", "aac", "-b:a", "256k"],
        ExportFormat::Wav24 => unreachable!(),
    };
    let status = std::process::Command::new(ffmpeg)
        .args(["-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(&tmp)
        .args(codec)
        .arg(path)
        .status();
    let _ = std::fs::remove_file(&tmp);
    anyhow::ensure!(status?.success(), "ffmpeg couldn't encode the file");
    Ok(())
}

/// Renders and writes the mixdown to `path` (picked by the UI).
#[tauri::command]
pub async fn export_mix(
    app: AppHandle,
    request: ExportRequest,
    path: PathBuf,
    progress: Channel<ExportProgress>,
) -> Result<(), String> {
    crate::produce::blocking(app, move |engine| {
        let (project, audio, takes) = sources(engine, &request.tracks)?;
        let song = project::track_audio(&audio);
        let tracks: Vec<TrackRef> = project
            .tracks
            .iter()
            .zip(&song)
            .map(|(t, a)| TrackRef {
                left: &a.left,
                right: &a.right,
                mix: t.mix,
            })
            .chain(project.takes.iter().zip(&takes).map(|(t, a)| TrackRef {
                left: a,
                right: a,
                mix: t.mix,
            }))
            .collect();
        let mut last = -1.0;
        let mix = export::mixdown(
            &tracks,
            &request.tracks,
            project.sample_rate,
            request.range,
            |f| {
                if f - last >= 0.02 || f >= 1.0 {
                    last = f;
                    let _ = progress.send(ExportProgress::Rendering { fraction: f });
                }
            },
        );
        let _ = progress.send(ExportProgress::Encoding);
        encode(&path, request.format, &export::encode_wav24(&mix))
    })
    .await
}

/// Writes the loaded song as a `.grain` archive at `path`.
#[tauri::command]
pub async fn export_archive(app: AppHandle, path: PathBuf) -> Result<(), String> {
    crate::produce::blocking(app, move |engine| {
        let (key, project) = with_loaded(engine, |l| Ok((l.cache_key.clone(), l.project.clone())))?;
        let Lookup::Hit { audio, takes, .. } = engine.cache.lookup(&key)? else {
            anyhow::bail!("this project is no longer on disk");
        };
        // The archive holds the open project (unsaved mix included) with the stored audio.
        let bytes = archive::encode(&project, &project::track_audio(&audio), &takes);
        Ok(std::fs::write(path, bytes)?)
    })
    .await
}

/// Opens a `.grain` archive: stores it as a saved project and makes it the loaded song.
#[tauri::command]
pub async fn import_archive(app: AppHandle, path: PathBuf) -> Result<Project, String> {
    crate::produce::blocking(app, move |engine| {
        let archive::Archive {
            mut project,
            tracks,
            takes,
        } = archive::decode(&std::fs::read(&path)?)?;
        let key = StemCache::key(&path, "archive")?;
        let audio = project::restore_audio(&mut project, tracks)?;
        engine.cache.store(&key, &project, &audio)?;
        for (t, timeline) in project.takes.iter().zip(&takes) {
            engine
                .cache
                .store_take(&key, t.id, timeline, project.sample_rate)?;
        }
        engine.cache.save_project(&key, &project)?;
        start_session(engine, key, project, &audio, takes)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_core::audio::StereoAudio;
    use engine_native::decode;

    /// Every desktop format decodes back to the mixdown it was made from (MP3 and M4A need
    /// ffmpeg; without it they are reported unavailable and the test checks only WAV).
    #[test]
    fn every_format_decodes_back_to_the_mixdown() {
        let sr = 44_100;
        let tone = |f: f32| {
            (0..sr as usize * 2)
                .map(move |i| 0.4 * (std::f32::consts::TAU * f * i as f32 / sr as f32).sin())
        };
        let mix = StereoAudio {
            sample_rate: sr,
            left: tone(440.0).collect(),
            right: tone(660.0).collect(),
        };
        let wav = export::encode_wav24(&mix);
        let dir = std::env::temp_dir().join(format!("grain-export-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let formats: Vec<ExportFormat> = export_formats()
            .into_iter()
            .filter(|(_, why)| why.is_none())
            .map(|(f, _)| f)
            .collect();
        assert!(formats.contains(&ExportFormat::Wav24));
        for format in formats {
            let path = dir.join(format!("mix.{}", format.extension()));
            encode(&path, format, &wav).unwrap();
            let back = decode::decode(&path).unwrap();
            assert_eq!(back.sample_rate, sr, "{format:?}");
            // Lossy codecs add priming delay and ringing: compare levels, and for WAV the samples.
            let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
            let want = rms(&mix.left);
            assert!(
                (rms(&back.left) - want).abs() < 0.02,
                "{format:?}: rms {} vs {want}",
                rms(&back.left)
            );
            assert!(
                (back.left.len() as i64 - mix.left.len() as i64).abs() < 4_096,
                "{format:?}: {} frames",
                back.left.len()
            );
            if format == ExportFormat::Wav24 {
                for i in (0..mix.left.len()).step_by(4_999) {
                    assert!((back.left[i] - mix.left[i]).abs() < 1e-5);
                    assert!((back.right[i] - mix.right[i]).abs() < 1e-5);
                }
            }
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
