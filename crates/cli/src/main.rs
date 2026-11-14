//! `music` CLI.

use anyhow::Result;
use clap::{Parser, Subcommand, ValueEnum};
use engine_core::note::{hz_to_midi, note_name};
use engine_core::pitch::{PitchRange, Yin};
use engine_core::project::{self, SongAudio, SongSource};
use engine_core::score::{Difficulty, NoteResult, OctaveMode, ScoreSettings};
use engine_core::WORKING_SAMPLE_RATE;
use engine_native::cache::StemCache;
use engine_native::decode;
use engine_native::import::{import_song, Imported, Separation};
use engine_native::separate::Demucs;
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
    /// Import a song: decode, separate (with --model), analyze, cache.
    Import {
        song: PathBuf,
        #[command(flatten)]
        engine: EngineArgs,
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
    /// HTDemucs ONNX graph (its .json sidecar must sit next to it). Omit to skip separation.
    #[arg(long)]
    model: Option<PathBuf>,
    #[arg(long, default_value = "stem-cache")]
    cache: PathBuf,
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
                "reference  {:?}, {} notes",
                a.reference_source,
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
            let report = project::score_take(&imported.project, &take, settings);
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
    }
}

fn import(song: &Path, engine: &EngineArgs) -> Result<Imported> {
    let mut model = engine.model.as_deref().map(Demucs::load).transpose()?;
    let separation = match model.as_mut() {
        Some(m) => Separation::Model(m),
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
