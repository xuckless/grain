//! In-memory WAV files for the generation sidecar's HTTP contract (AGENTS.md §8): 16-bit mono
//! out, any PCM (16/24/32-bit integer or 32-bit float, plain or extensible) in. Pure, so the
//! desktop and web hosts share it.

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum WavError {
    #[error("not a WAV file")]
    NotWav,
    #[error("unsupported WAV format {format} with {bits} bits")]
    Unsupported { format: u16, bits: u16 },
}

/// A 16-bit mono WAV file of `audio`.
pub fn encode_mono16(audio: &[f32], sample_rate: u32) -> Vec<u8> {
    let data = (audio.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + data as usize);
    out.extend(b"RIFF");
    out.extend((36 + data).to_le_bytes());
    out.extend(b"WAVEfmt ");
    out.extend(16u32.to_le_bytes());
    out.extend(1u16.to_le_bytes()); // PCM
    out.extend(1u16.to_le_bytes()); // mono
    out.extend(sample_rate.to_le_bytes());
    out.extend((sample_rate * 2).to_le_bytes());
    out.extend(2u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend(b"data");
    out.extend(data.to_le_bytes());
    for v in audio {
        out.extend(((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    out
}

/// A WAV file's audio with its channels averaged, and its sample rate.
pub fn decode_mono(bytes: &[u8]) -> Result<(Vec<f32>, u32), WavError> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(WavError::NotWav);
    }
    let u16_at = |b: &[u8], i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    let u32_at = |b: &[u8], i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]);
    let (mut fmt, mut data) = (None, None);
    let mut pos = 12;
    while pos + 8 <= bytes.len() {
        let len = u32_at(bytes, pos + 4) as usize;
        let body = &bytes[pos + 8..(pos + 8 + len).min(bytes.len())];
        match &bytes[pos..pos + 4] {
            b"fmt " if body.len() >= 16 => fmt = Some(body),
            b"data" => data = Some(body),
            _ => {}
        }
        pos += 8 + len + (len & 1);
    }
    let (Some(fmt), Some(data)) = (fmt, data) else {
        return Err(WavError::NotWav);
    };
    let mut format = u16_at(fmt, 0);
    let channels = usize::from(u16_at(fmt, 2).max(1));
    let rate = u32_at(fmt, 4);
    let bits = u16_at(fmt, 14);
    // WAVE_FORMAT_EXTENSIBLE: the real format is the sub-format GUID's first two bytes.
    if format == 0xFFFE && fmt.len() >= 26 {
        format = u16_at(fmt, 24);
    }
    let width = usize::from(bits / 8);
    let sample = |s: &[u8]| -> f32 {
        match (format, bits) {
            (3, 32) => f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
            (1, 16) => f32::from(i16::from_le_bytes([s[0], s[1]])) / 32768.0,
            (1, 24) => (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) as f32 / 8_388_608.0,
            (1, 32) => i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2_147_483_648.0,
            _ => 0.0,
        }
    };
    if !matches!((format, bits), (3, 32) | (1, 16) | (1, 24) | (1, 32)) {
        return Err(WavError::Unsupported { format, bits });
    }
    let frame = width * channels;
    let mono = data
        .chunks_exact(frame)
        .map(|f| f.chunks_exact(width).map(sample).sum::<f32>() / channels as f32)
        .collect();
    Ok((mono, rate))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_arch = "wasm32")]
    use wasm_bindgen_test::wasm_bindgen_test as test;

    #[test]
    fn round_trips_16_bit_mono() {
        let x: Vec<f32> = (0..1000).map(|i| (i as f32 / 100.0).sin() * 0.5).collect();
        let (y, rate) = decode_mono(&encode_mono16(&x, 22_050)).unwrap();
        assert_eq!(rate, 22_050);
        assert_eq!(y.len(), x.len());
        assert!(x.iter().zip(&y).all(|(a, b)| (a - b).abs() < 1e-4));
    }

    #[test]
    fn averages_float_stereo_and_rejects_junk() {
        let mut w = encode_mono16(&[], 48_000);
        // Rewrite as 32-bit float stereo with one frame (0.5, -0.25).
        w[20..22].copy_from_slice(&3u16.to_le_bytes());
        w[22..24].copy_from_slice(&2u16.to_le_bytes());
        w[34..36].copy_from_slice(&32u16.to_le_bytes());
        w[40..44].copy_from_slice(&8u32.to_le_bytes());
        w.extend(0.5f32.to_le_bytes());
        w.extend((-0.25f32).to_le_bytes());
        assert_eq!(decode_mono(&w).unwrap(), (vec![0.125], 48_000));
        assert_eq!(decode_mono(b"nope"), Err(WavError::NotWav));
    }
}
