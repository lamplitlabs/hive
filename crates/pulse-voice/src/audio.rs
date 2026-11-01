//! Audio in: what the model takes is mono 16 kHz samples in `[-1, 1]`. The
//! console records at 16 kHz and posts the raw little-endian 16-bit PCM; a
//! WAV from the command line is decoded and, when it is at another rate or
//! has several channels, mixed down and resampled.

use anyhow::{Result, anyhow, bail};
use std::io::Cursor;

/// The one sample rate the models take.
pub const SAMPLE_RATE: u32 = 16_000;

/// The longest utterance one request takes: five minutes.
pub const MAX_SECONDS: u32 = 300;

/// Bytes of 16-bit mono PCM at the model's rate for [`MAX_SECONDS`].
pub const MAX_PCM_BYTES: usize = (MAX_SECONDS * SAMPLE_RATE * 2) as usize;

/// Raw little-endian 16-bit mono PCM at 16 kHz, as the console posts it.
pub fn pcm16_to_samples(bytes: &[u8]) -> Result<Vec<f32>> {
    if !bytes.len().is_multiple_of(2) {
        bail!(
            "the audio holds {} bytes, which is not whole 16-bit samples",
            bytes.len()
        );
    }
    if bytes.len() > MAX_PCM_BYTES {
        bail!(
            "the audio is {} seconds; one recording takes up to {MAX_SECONDS}",
            bytes.len() / 2 / SAMPLE_RATE as usize
        );
    }
    Ok(bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0)
        .collect())
}

/// What a decoded WAV said about itself, for the command line's report.
#[derive(Clone, Debug, PartialEq)]
pub struct WavInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub seconds: f32,
}

/// A WAV file's bytes as mono 16 kHz samples.
pub fn wav_to_samples(bytes: &[u8]) -> Result<(Vec<f32>, WavInfo)> {
    let mut reader = hound::WavReader::new(Cursor::new(bytes))
        .map_err(|e| anyhow!("the file is not a WAV pulse can read: {e}"))?;
    let spec = reader.spec();
    if spec.channels == 0 || spec.sample_rate == 0 {
        bail!(
            "the WAV header names {} channels at {} Hz",
            spec.channels,
            spec.sample_rate
        );
    }
    let interleaved: Vec<f32> = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, 32) => reader
            .samples::<f32>()
            .collect::<std::result::Result<_, _>>()
            .map_err(|e| anyhow!("the WAV's float samples do not read: {e}"))?,
        (hound::SampleFormat::Int, bits) if (8..=32).contains(&bits) => {
            let scale = 1.0 / ((1i64 << (bits - 1)) as f32);
            reader
                .samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<std::result::Result<_, _>>()
                .map_err(|e| anyhow!("the WAV's samples do not read: {e}"))?
        }
        (format, bits) => bail!("the WAV is {bits}-bit {format:?}, which pulse does not read"),
    };
    let channels = spec.channels as usize;
    let mono: Vec<f32> = if channels == 1 {
        interleaved
    } else {
        interleaved
            .chunks(channels)
            .map(|frame| frame.iter().sum::<f32>() / channels as f32)
            .collect()
    };
    let seconds = mono.len() as f32 / spec.sample_rate as f32;
    if seconds > MAX_SECONDS as f32 {
        bail!("the audio is {seconds:.0} seconds; one recording takes up to {MAX_SECONDS}");
    }
    let samples = resample(&mono, spec.sample_rate, SAMPLE_RATE);
    Ok((
        samples,
        WavInfo {
            sample_rate: spec.sample_rate,
            channels: spec.channels,
            seconds,
        },
    ))
}

/// Linear resampling: enough for speech going to a 16 kHz recognizer from
/// the 44.1/48 kHz a sound card records at.
pub fn resample(samples: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || samples.is_empty() {
        return samples.to_vec();
    }
    let ratio = from as f64 / to as f64;
    let out_len = ((samples.len() as f64) / ratio).floor().max(1.0) as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let pos = i as f64 * ratio;
        let idx = pos.floor() as usize;
        let frac = (pos - idx as f64) as f32;
        let a = samples[idx.min(samples.len() - 1)];
        let b = samples[(idx + 1).min(samples.len() - 1)];
        out.push(a + (b - a) * frac);
    }
    out
}

/// Seconds of audio in `samples` at the model's rate.
pub fn seconds(samples: &[f32]) -> f32 {
    samples.len() as f32 / SAMPLE_RATE as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(rate: u32, channels: u16, frames: &[i16]) -> Vec<u8> {
        let spec = hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut cur = Cursor::new(Vec::new());
        {
            let mut w = hound::WavWriter::new(&mut cur, spec).unwrap();
            for s in frames {
                w.write_sample(*s).unwrap();
            }
            w.finalize().unwrap();
        }
        cur.into_inner()
    }

    #[test]
    fn pcm16_scales_and_refuses_odd_lengths() {
        let s = pcm16_to_samples(&[0, 0, 0xff, 0x7f, 0, 0x80]).unwrap();
        assert_eq!(s.len(), 3);
        assert_eq!(s[0], 0.0);
        assert!((s[1] - 0.99997).abs() < 1e-4);
        assert_eq!(s[2], -1.0);
        let err = pcm16_to_samples(&[1]).unwrap_err().to_string();
        assert!(err.contains("not whole 16-bit samples"), "{err}");
        let err = pcm16_to_samples(&vec![0u8; MAX_PCM_BYTES + 2])
            .unwrap_err()
            .to_string();
        assert!(err.contains("up to 300"), "{err}");
    }

    #[test]
    fn wav_mixes_down_and_resamples() {
        // Stereo 48 kHz, half a second: left loud, right silent.
        let frames: Vec<i16> = (0..24_000).flat_map(|_| [16384i16, 0]).collect();
        let (s, info) = wav_to_samples(&wav(48_000, 2, &frames)).unwrap();
        assert_eq!(
            info,
            WavInfo {
                sample_rate: 48_000,
                channels: 2,
                seconds: 0.5
            }
        );
        assert_eq!(s.len(), 8_000, "half a second at 16 kHz");
        assert!(
            s.iter().all(|v| (v - 0.25).abs() < 1e-5),
            "mixed to a quarter"
        );
        assert!((seconds(&s) - 0.5).abs() < 1e-6);
        // Mono 16 kHz passes straight through.
        let (s, _) = wav_to_samples(&wav(16_000, 1, &[100, -100, 0])).unwrap();
        assert_eq!(s.len(), 3);
        let err = wav_to_samples(b"not a wav").unwrap_err().to_string();
        assert!(err.contains("not a WAV"), "{err}");
    }

    #[test]
    fn resample_keeps_length_ratio_and_endpoints() {
        let up = resample(&[0.0, 1.0], 1, 4);
        assert_eq!(up.len(), 8);
        assert_eq!(up[0], 0.0);
        assert!((up[4] - 1.0).abs() < 1e-6);
        let same = resample(&[0.5; 10], 16_000, 16_000);
        assert_eq!(same.len(), 10);
        assert!(resample(&[], 48_000, 16_000).is_empty());
    }
}
