//! File decoding (symphonia) and resampling (rubato) to the working sample rate.

use anyhow::{bail, Context, Result};
use engine_core::audio::StereoAudio;
use engine_core::project::SongAudio;
use rubato::{FftFixedIn, Resampler};
use std::fs::File;
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Decodes an audio file and resamples it to `sample_rate`.
pub fn load(path: &Path, sample_rate: u32) -> Result<StereoAudio> {
    let decoded = decode(path).with_context(|| format!("decoding {}", path.display()))?;
    resample(&decoded, sample_rate)
}

/// Decodes the first audio track; mono is duplicated, extra channels are dropped.
pub fn decode(path: &Path) -> Result<StereoAudio> {
    let file = File::open(path)?;
    let stream = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe().format(
        &hint,
        stream,
        &FormatOptions::default(),
        &MetadataOptions::default(),
    )?;
    let mut format = probed.format;
    let track = format.default_track().context("no audio track")?.clone();
    let mut decoder =
        symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default())?;
    let sample_rate = track
        .codec_params
        .sample_rate
        .context("unknown sample rate")?;

    let (mut left, mut right) = (Vec::new(), Vec::new());
    let mut buffer: Option<SampleBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                break
            }
            Err(e) => return Err(e.into()),
        };
        if packet.track_id() != track.id {
            continue;
        }
        let audio = match decoder.decode(&packet) {
            Ok(a) => a,
            // Corrupt frames in lossy streams are skipped, as players do.
            Err(SymphoniaError::DecodeError(_)) => continue,
            Err(e) => return Err(e.into()),
        };
        let spec = *audio.spec();
        let buf = buffer.get_or_insert_with(|| SampleBuffer::new(audio.capacity() as u64, spec));
        if buf.capacity() < audio.capacity() {
            *buf = SampleBuffer::new(audio.capacity() as u64, spec);
        }
        buf.copy_interleaved_ref(audio);
        let channels = spec.channels.count();
        for frame in buf.samples().chunks(channels) {
            left.push(frame[0]);
            right.push(*frame.get(1).unwrap_or(&frame[0]));
        }
    }
    if left.is_empty() {
        bail!("no audio decoded");
    }
    Ok(StereoAudio {
        sample_rate,
        left,
        right,
    })
}

pub fn resample(audio: &StereoAudio, sample_rate: u32) -> Result<StereoAudio> {
    if audio.sample_rate == sample_rate {
        return Ok(audio.clone());
    }
    const CHUNK: usize = 4096;
    let mut resampler = FftFixedIn::<f32>::new(
        audio.sample_rate as usize,
        sample_rate as usize,
        CHUNK,
        2,
        2,
    )?;
    let delay = resampler.output_delay();
    let expected = (audio.len() as u64 * sample_rate as u64 / audio.sample_rate as u64) as usize;
    let mut out = [
        Vec::with_capacity(expected + CHUNK),
        Vec::with_capacity(expected + CHUNK),
    ];
    let mut pos = 0;
    while pos < audio.len() {
        let end = (pos + CHUNK).min(audio.len());
        let mut chunk = [
            audio.left[pos..end].to_vec(),
            audio.right[pos..end].to_vec(),
        ];
        chunk.iter_mut().for_each(|c| c.resize(CHUNK, 0.0));
        let produced = resampler.process(&chunk, None)?;
        for (o, p) in out.iter_mut().zip(produced) {
            o.extend(p);
        }
        pos = end;
    }
    // Flush the filter delay with silence.
    while out[0].len() < expected + delay {
        let silence = vec![vec![0.0f32; CHUNK]; 2];
        for (o, p) in out.iter_mut().zip(resampler.process(&silence, None)?) {
            o.extend(p);
        }
    }
    let [left, right] = out.map(|o| o[delay..delay + expected].to_vec());
    Ok(StereoAudio {
        sample_rate,
        left,
        right,
    })
}

pub fn resample_song(audio: &SongAudio, sample_rate: u32) -> Result<SongAudio> {
    Ok(match audio {
        SongAudio::Unseparated { mix } => SongAudio::Unseparated {
            mix: resample(mix, sample_rate)?,
        },
        SongAudio::Separated { stems } => SongAudio::Separated {
            stems: stems
                .iter()
                .map(|(s, a)| Ok((*s, resample(a, sample_rate)?)))
                .collect::<Result<_>>()?,
        },
    })
}

/// Writes 32-bit float stereo WAV.
pub fn write_wav(path: &Path, audio: &StereoAudio) -> Result<()> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: audio.sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec)?;
    for (l, r) in audio.left.iter().zip(&audio.right) {
        writer.write_sample(*l)?;
        writer.write_sample(*r)?;
    }
    writer.finalize()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wav_round_trip_and_resample_preserve_pitch_and_length() {
        let sr = 48_000;
        let tone: Vec<f32> = (0..sr)
            .map(|i| 0.5 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / sr as f32).sin())
            .collect();
        let audio = StereoAudio {
            sample_rate: sr,
            left: tone.clone(),
            right: tone,
        };
        let dir = std::env::temp_dir().join(format!("engine-native-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tone.wav");
        write_wav(&path, &audio).unwrap();
        let back = decode(&path).unwrap();
        assert_eq!(back, audio);

        let resampled = load(&path, 44_100).unwrap();
        assert_eq!(resampled.len(), 44_100);
        let track = engine_core::pitch::track(&resampled.to_mono(), 44_100);
        let engine_core::pitch::PitchFrame::Voiced { midi } = track.frames[50] else {
            panic!("unvoiced");
        };
        assert!((midi - 69.0).abs() < 0.02, "midi {midi}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
