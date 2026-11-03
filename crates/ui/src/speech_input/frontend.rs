//! Original Whisper frontend: centered periodic-Hann STFT and Slaney mel powers.
//! A mixed-radix FFT keeps the reference power convention (no doubling of
//! positive-frequency bins). Zero-padded frames are skipped.
const WINDOW: usize = 400;
const BINS: usize = WINDOW / 2 + 1;
#[cfg(test)]
const FRAMES: usize = 3000;
const SAMPLES: usize = 480_000;
pub(super) struct Frontend {
    window: Vec<f32>,
    fft: super::fft::Fft,
    filters: Vec<f32>,
    bands: usize,
}
impl Frontend {
    pub fn new(bands: usize) -> Self {
        let window = (0..WINDOW)
            .map(|i| (0.5 - 0.5 * (std::f64::consts::TAU * i as f64 / WINDOW as f64).cos()) as f32)
            .collect();
        Self {
            window,
            fft: super::fft::Fft::new(WINDOW),
            filters: mel_filters(bands),
            bands,
        }
    }
    pub fn mel(&self, pcm: &[f32], frames: usize) -> Vec<f32> {
        let mut mel = vec![-10f32; self.bands * frames];
        let end = (pcm.len().div_ceil(160) + 2).min(frames);
        let mut windowed = vec![super::fft::Complex::default(); WINDOW];
        let mut transformed = windowed.clone();
        let mut scratch = windowed.clone();
        for frame in 0..end {
            for (i, (value, window)) in windowed.iter_mut().zip(&self.window).enumerate() {
                let at = (frame * 160 + i) as isize - (WINDOW / 2) as isize;
                // Reflect around the ends of the zero-padded 30-second signal.
                let at = if at < 0 {
                    -at
                } else if at >= SAMPLES as isize {
                    (2 * SAMPLES - 2) as isize - at
                } else {
                    at
                };
                value.re = pcm.get(at as usize).copied().unwrap_or(0.) * window;
            }
            self.fft.forward(&windowed, &mut transformed, &mut scratch);
            let powers: Vec<f32> = transformed
                .iter()
                .take(BINS)
                .map(|v| v.re * v.re + v.im * v.im)
                .collect();
            for (band, filter) in self.filters.as_chunks::<BINS>().0.iter().enumerate() {
                let power: f32 = powers.iter().zip(filter).map(|(p, f)| p * f).sum();
                if let Some(value) = mel.get_mut(band * frames + frame) {
                    *value = power.max(1e-10).log10();
                }
            }
        }
        let floor = mel.iter().copied().fold(f32::NEG_INFINITY, f32::max) - 8.;
        for value in &mut mel {
            *value = (value.max(floor) + 4.) / 4.;
        }
        mel
    }
}
/// Slaney triangular mel bank (area normalized), 80/128 bands over 0–8 kHz.
/// Constructed from the filter definition rather than copied binary assets.
fn mel_filters(bands: usize) -> Vec<f32> {
    let hz = |mel: f64| {
        if mel < 15. {
            mel * (200. / 3.)
        } else {
            1000. * ((mel - 15.) * (6.4f64.ln() / 27.)).exp()
        }
    };
    let max_mel = 15. + (8000f64 / 1000.).ln() / (6.4f64.ln() / 27.);
    let edges: Vec<_> = (0..bands + 2)
        .map(|i| hz(max_mel * i as f64 / (bands + 1) as f64))
        .collect();
    let mut filters = Vec::with_capacity(bands * BINS);
    for window in edges.windows(3) {
        let [low, center, high] = window else {
            continue;
        };
        for bin in 0..BINS {
            let frequency = bin as f64 * 40.;
            filters.push(
                (((frequency - low) / (center - low))
                    .min((high - frequency) / (high - center))
                    .max(0.)
                    * 2.
                    / (high - low)) as f32,
            );
        }
    }
    filters
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn centered_impulse_has_unit_single_sided_power_and_silence_is_finite() {
        let frontend = Frontend::new(80);
        let silent = frontend.mel(&vec![0.; 16000], FRAMES);
        assert_eq!(silent.len(), 80 * FRAMES);
        assert!(silent.iter().all(|v| (*v + 1.5).abs() < 1e-6));
        let mut impulse = vec![0.; 16000];
        if let Some(first) = impulse.first_mut() {
            *first = 1.;
        }
        let mel = frontend.mel(&impulse, FRAMES);
        for (band, filter) in frontend.filters.as_chunks::<BINS>().0.iter().enumerate() {
            // A centered unit impulse sees Hann(200)=1 and power 1 in every bin.
            let expected = (filter.iter().sum::<f32>().max(1e-10).log10() + 4.) / 4.;
            let value = mel.get(band * FRAMES).copied().unwrap_or(f32::NAN);
            assert!(
                (value - expected).abs() < 1e-5,
                "band {band}: {value} != {expected}"
            );
        }
    }
}
