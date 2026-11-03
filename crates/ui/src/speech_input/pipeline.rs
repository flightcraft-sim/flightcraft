//! Bounded PCM ingestion shared by microphone and WAV regression tests.
#[derive(Clone, Debug)]
pub struct AudioClip {
    pub samples: Vec<f32>,
    pub rate: u32,
}
#[derive(Clone, Debug)]
pub struct Recognized {
    pub text: String,
    pub confidence: f32,
    pub latency_ms: u64,
    /// Ranked hypotheses actually emitted by the recognizer, never expected answers.
    pub alternatives: Vec<(String, f32)>,
}
pub trait Recognizer {
    fn set_context(&mut self, _context: &str) -> Result<(), String> {
        Ok(())
    }
    fn recognize(&mut self, clip: &AudioClip) -> Result<Recognized, String>;
}
impl AudioClip {
    pub fn mono_16k(&self) -> Result<Vec<f32>, String> {
        if !(8000..=192000).contains(&self.rate)
            || self.samples.len() > self.rate as usize * 30
            || self.samples.is_empty()
            || self.samples.iter().any(|v| !v.is_finite())
        {
            return Err("Use a recording of 30 seconds or less with a supported sample rate.".into());
        }
        // Multiply in u64 so a valid 30-second, 192 kHz input cannot
        // overflow usize on wasm32. The validated result is at most 480,000.
        let length = (self.samples.len() as u64 * 16000 / u64::from(self.rate)) as usize;
        Ok((0..length)
            .map(|i| {
                let at = i as f64 * f64::from(self.rate) / 16000.;
                let lower = at as usize;
                let fraction = (at - lower as f64) as f32;
                self.samples.get(lower).copied().unwrap_or(0.) * (1. - fraction)
                    + self.samples.get(lower + 1).copied().unwrap_or(0.) * fraction
            })
            .collect())
    }
    pub fn from_wave(bytes: &[u8]) -> Result<Self, String> {
        let invalid = || "Use a PCM 16-bit mono or stereo WAV, up to 30 seconds.".to_owned();
        if bytes.len() > 24 * 1024 * 1024
            || bytes.get(..4) != Some(b"RIFF")
            || bytes.get(8..12) != Some(b"WAVE")
        {
            return Err(invalid());
        }
        let mut at = 12usize;
        let mut format = None;
        let mut pcm = None;
        while let Some(header) = bytes.get(at..at.saturating_add(8)) {
            let size = u32::from_le_bytes(
                header
                    .get(4..8)
                    .and_then(|v| v.try_into().ok())
                    .ok_or_else(invalid)?,
            ) as usize;
            at += 8;
            let chunk = bytes
                .get(at..at.checked_add(size).ok_or_else(invalid)?)
                .ok_or_else(invalid)?;
            if header.get(..4) == Some(b"fmt ") {
                let u16_at = |at| {
                    chunk
                        .get(at..at + 2)
                        .and_then(|v| v.try_into().ok())
                        .map(u16::from_le_bytes)
                };
                let rate = chunk
                    .get(4..8)
                    .and_then(|v| v.try_into().ok())
                    .map(u32::from_le_bytes)
                    .ok_or_else(invalid)?;
                if u16_at(0) != Some(1) || u16_at(14) != Some(16) {
                    return Err(invalid());
                }
                let channels = u16_at(2).filter(|c| (1..=2).contains(c)).ok_or_else(invalid)?;
                if u16_at(12) != Some(channels * 2) {
                    return Err(invalid());
                }
                format = Some((channels, rate));
            } else if header.get(..4) == Some(b"data") {
                pcm = Some(chunk);
            }
            at = at.checked_add(size + size % 2).ok_or_else(invalid)?;
        }
        let (channels, rate) = format.ok_or_else(invalid)?;
        let pcm = pcm.ok_or_else(invalid)?;
        if !(8000..=192000).contains(&rate)
            || pcm.len() % (usize::from(channels) * 2) != 0
            || pcm.len() > rate as usize * usize::from(channels) * 2 * 30
        {
            return Err(invalid());
        }
        let samples = pcm
            .chunks_exact(usize::from(channels) * 2)
            .map(|frame| {
                frame
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .copied()
                    .map(i16::from_le_bytes)
                    .map(|v| f32::from(v) / 32768.)
                    .sum::<f32>()
                    / f32::from(channels)
            })
            .collect();
        Ok(Self { samples, rate })
    }
}
