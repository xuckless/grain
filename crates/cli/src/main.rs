//! `music` CLI.

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use engine_core::note::{hz_to_midi, note_name};
use engine_core::pitch::PitchFrame;
use engine_core::pitch::{PitchRange, Yin};
use engine_core::project::{
    self, PitchTracker, SeparationQuality, SongAudio, SongSource, TrackedPitch,
};
use engine_core::score::{Difficulty, NoteResult, OctaveMode, ScoreSettings};
use engine_core::WORKING_SAMPLE_RATE;
use engine_native::cache::StemCache;
use engine_native::decode;
use engine_native::import::{import_song, Imported, Separation};
use engine_native::pitch::Pitch;
use engine_native::separate::Separators;
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "music", about = "Music workstation engine CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Port of prototype/main.py: per-second YIN pitch of two recordings.
    Compare { mine: PathBuf, artist: PathBuf },
    /// Import a song: decode, separate (with --models), analyze, cache.
    Import {
        song: PathBuf,
        #[command(flatten)]
        engine: EngineArgs,
    },
    /// Pitch of a (vocal) recording with YIN and RMVPE side by side: agreement and coverage.
    Pitch {
        recording: PathBuf,
        /// Directory holding rmvpe.onnx.
        #[arg(long)]
        models: PathBuf,
    },
    /// Score a sung take against a song's reference vocal.
    Score {
        song: PathBuf,
        take: PathBuf,
        #[command(flatten)]
        engine: EngineArgs,
        #[arg(long, value_enum, default_value = "medium")]
        difficulty: DifficultyArg,
        /// Penalize singing in a different octave.
        #[arg(long)]
        strict_octave: bool,
    },
}

#[derive(clap::Args)]
struct EngineArgs {
    /// Directory holding the separation weights (models/weights layout: htdemucs_6s, plus
    /// melband_roformer_vocals for the vocal cascade). Omit to skip separation.
    #[arg(long)]
    models: Option<PathBuf>,
    #[arg(long, default_value = "stem-cache")]
    cache: PathBuf,
    /// Separation to run when --models is given.
    #[arg(long, value_enum, default_value = "fast")]
    quality: QualityArg,
}

#[derive(Clone, Copy, ValueEnum)]
enum QualityArg {
    Fast,
    BestVocals,
}

#[derive(Clone, Copy, ValueEnum)]
enum DifficultyArg {
    Easy,
    Medium,
    Hard,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Compare { mine, artist } => compare(&mine, &artist),
        Command::Import { song, engine } => {
            let imported = import(&song, &engine)?;
            let a = &imported.project.analysis;
            println!("cache key  {}", imported.cache_key);
            println!(
                "tracks     {}",
                imported
                    .project
                    .tracks
                    .iter()
                    .map(|t| t.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            println!(
                "tempo      {:.1} BPM ({} beats)",
                a.tempo.bpm,
                a.tempo.beats.len()
            );
            for k in &a.keys {
                println!("key        {} (r = {:.2})", k.label, k.correlation);
            }
            println!(
                "reference  {:?} ({:?}), {} notes",
                a.reference_source,
                a.pitch_tracker,
                a.reference_notes.len()
            );
            if let SongAudio::Separated { stems } = &imported.audio {
                let mix = decode::load(&song, WORKING_SAMPLE_RATE)?;
                let mut residual = mix.clone();
                for (stem, audio) in stems {
                    println!("stem       {stem:?}: {:.1} dBFS", audio.rms_db());
                    for (r, s) in residual
                        .left
                        .iter_mut()
                        .zip(&audio.left)
                        .chain(residual.right.iter_mut().zip(&audio.right))
                    {
                        *r -= s;
                    }
                }
                println!(
                    "residual   {:.1} dB below the mix",
                    mix.rms_db() - residual.rms_db()
                );
            }
            Ok(())
        }
        Command::Score {
            song,
            take,
            engine,
            difficulty,
            strict_octave,
        } => {
            let imported = import(&song, &engine)?;
            let take = decode::load(&take, WORKING_SAMPLE_RATE)?;
            let take = pitch_trackers(&engine).track(
                imported.project.analysis.pitch_tracker,
                &take.to_mono(),
                WORKING_SAMPLE_RATE,
            )?;
            let settings = ScoreSettings {
                difficulty: match difficulty {
                    DifficultyArg::Easy => Difficulty::Easy,
                    DifficultyArg::Medium => Difficulty::Medium,
                    DifficultyArg::Hard => Difficulty::Hard,
                },
                octave: if strict_octave {
                    OctaveMode::Strict
                } else {
                    OctaveMode::Fold
                },
            };
            let report = project::score_take(&imported.project, &take, settings)?;
            let s = report.scores;
            println!("offset     {:+.2} s", report.offset_seconds);
            println!("overall    {:.1}", s.overall);
            println!(
                "pitch      {:.1}\ntiming     {:.1}\nstability  {:.1}\ncoverage   {:.1}",
                s.pitch, s.timing, s.stability, s.coverage
            );
            let missed = report
                .notes
                .iter()
                .filter(|n| n.result == NoteResult::Missed)
                .count();
            println!(
                "notes      {} sung, {missed} missed",
                report.notes.len() - missed
            );
            Ok(())
        }
        Command::Pitch { recording, models } => {
            let audio = decode::load(&recording, WORKING_SAMPLE_RATE)?.to_mono();
            let yin = TrackedPitch::yin(&audio, WORKING_SAMPLE_RATE).track;
            let started = std::time::Instant::now();
            let rmvpe = Pitch::find(std::slice::from_ref(&models))
                .track(PitchTracker::Rmvpe, &audio, WORKING_SAMPLE_RATE)?
                .track;
            let secs = started.elapsed().as_secs_f64();
            let voiced = |t: &[PitchFrame]| {
                t.iter()
                    .filter(|f| matches!(f, PitchFrame::Voiced { .. }))
                    .count()
            };
            let (mut both, mut within) = (0usize, 0usize);
            for (a, b) in yin.frames.iter().zip(&rmvpe.frames) {
                if let (PitchFrame::Voiced { midi: y }, PitchFrame::Voiced { midi: r }) = (a, b) {
                    both += 1;
                    if (100.0 * (y - r)).abs() <= 50.0 {
                        within += 1;
                    }
                }
            }
            println!(
                "frames     {} (YIN) / {} (RMVPE), 10 ms",
                yin.frames.len(),
                rmvpe.frames.len()
            );
            println!(
                "voiced     YIN {} / RMVPE {}",
                voiced(&yin.frames),
                voiced(&rmvpe.frames)
            );
            println!(
                "agreement  {within} of {both} frames voiced in both within 50 cents ({:.1}%)",
                100.0 * within as f64 / both.max(1) as f64
            );
            println!("rmvpe      {secs:.1} s");
            Ok(())
        }
    }
}

/// RMVPE when --models holds it, YIN otherwise.
fn pitch_trackers(engine: &EngineArgs) -> Pitch {
    match &engine.models {
        Some(dir) => Pitch::find(std::slice::from_ref(dir)),
        None => Pitch::YinOnly,
    }
}

fn import(song: &Path, engine: &EngineArgs) -> Result<Imported> {
    let mut model = match &engine.models {
        Some(dir) => Some(
            Separators::find(std::slice::from_ref(dir))?
                .ok_or_else(|| anyhow::anyhow!("no separation weights in {}", dir.display()))?,
        ),
        None => None,
    };
    let separation = match model.as_mut() {
        Some(separators) => Separation::Model {
            separators,
            quality: match engine.quality {
                QualityArg::Fast => SeparationQuality::Fast,
                QualityArg::BestVocals => SeparationQuality::BestVocals,
            },
        },
        None => Separation::Disabled,
    };
    let name = song
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    import_song(
        song,
        SongSource::File { name },
        separation,
        &mut pitch_trackers(engine),
        &StemCache::new(engine.cache.clone()),
        |stage| eprintln!("{stage:?}"),
    )
}

/// Same procedure as the prototype: native sample rate, mono, 1 s frames, YIN 60–1000 Hz.
fn compare(mine: &Path, artist: &Path) -> Result<()> {
    const PROTOTYPE_RANGE: PitchRange = PitchRange {
        min_hz: 60.0,
        max_hz: 1000.0,
    };
    let per_second = |path: &Path| -> Result<(Vec<Option<f32>>, u32)> {
        let audio = decode::decode(path)?;
        let sr = audio.sample_rate;
        let mono = audio.to_mono();
        let mut yin = Yin::new(sr as usize);
        let values = mono
            .chunks_exact(sr as usize)
            .map(|frame| yin.estimate(frame, sr, PROTOTYPE_RANGE).map(|e| e.hz))
            .collect();
        Ok((values, sr))
    };
    let (m, _) = per_second(mine)?;
    let (a, _) = per_second(artist)?;
    let fmt = |v: Option<f32>| match v {
        Some(hz) => format!("{hz:8.1} Hz {:<4}", note_name(hz_to_midi(hz))),
        None => format!("{:>16}", "—"),
    };
    println!("{:>4}  {:>16}  {:>16}", "sec", "artist", "mine");
    for (i, (av, mv)) in a.iter().zip(&m).enumerate() {
        println!("{i:>4}  {}  {}", fmt(*av), fmt(*mv));
    }
    Ok(())
}
