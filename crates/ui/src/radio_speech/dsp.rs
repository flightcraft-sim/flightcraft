//! Original VHF radio character: squelch break, band limit, noise that grows with distance, carrier
//! clicks, and the heterodyne squeal of two transmitters on one frequency. Pure and deterministic.
use flightcraft_control::RadioCharacter;

/// How one received transmission sounds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RadioParams {
    pub radio: RadioCharacter,
    /// 0 at the antenna, 1 at the edge of service range.
    pub distance: f32,
    /// Varies the static between transmissions; stable for one event.
    pub seed: u32,
}
impl RadioParams {
    /// Noise floor: the radio's hiss, plus a term that grows with the square of the distance.
    pub fn noise_level(&self) -> f32 {
        let d = self.distance.clamp(0.0, 1.0);
        self.radio.hiss + 0.07 * d * d
    }
}

/// Squelch opening burst before the voice.
pub const SQUELCH_BREAK_S: f32 = 0.045;
/// Squelch closing tail after the voice.
pub const SQUELCH_TAIL_S: f32 = 0.12;

pub fn noise(index: usize, seed: u32) -> f32 {
    let mut bits = (index as u32)
        .wrapping_mul(0x9e37_79b9)
        .wrapping_add(0x7f4a_7c15)
        .wrapping_add(seed.wrapping_mul(0x85eb_ca6b));
    bits ^= bits >> 16;
    bits = bits.wrapping_mul(0x85eb_ca6b);
    bits ^= bits >> 13;
    bits = bits.wrapping_mul(0xc2b2_ae35);
    bits ^= bits >> 16;
    (bits & 65535) as f32 / 32767.5 - 1.0
}

/// Two one-pole low-pass sections and one high-pass section: roughly 300 to 3000 Hz.
struct Band {
    hp: f32,
    lp: f32,
    hp_state: f32,
    previous: f32,
    lp1: f32,
    lp2: f32,
}
impl Band {
    fn new(rate: f64, low_hz: f32, high_hz: f32) -> Self {
        Self {
            hp: libm::exp(-core::f64::consts::TAU * f64::from(low_hz) / rate) as f32,
            lp: 1.0 - libm::exp(-core::f64::consts::TAU * f64::from(high_hz) / rate) as f32,
            hp_state: 0.0,
            previous: 0.0,
            lp1: 0.0,
            lp2: 0.0,
        }
    }
    fn run(&mut self, input: f32) -> f32 {
        self.hp_state = self.hp * (self.hp_state + input - self.previous);
        self.previous = input;
        self.lp1 += self.lp * (self.hp_state - self.lp1);
        self.lp2 += self.lp * (self.lp1 - self.lp2);
        self.lp2
    }
}

fn soft_clip(v: f32, drive: f32) -> f32 {
    let x = v * drive;
    x / (1.0 + x.abs())
}

/// Wraps `samples` in the radio: squelch break, band-limited and compressed voice over a noise floor
/// that rises with distance, a weaker voice at range, and the squelch tail. Grows the buffer by
/// `SQUELCH_BREAK_S + SQUELCH_TAIL_S`.
pub fn radio_effect(samples: &mut Vec<f32>, rate: u32, params: &RadioParams) {
    let rate = rate.max(8000);
    let r = f64::from(rate);
    let lead = (SQUELCH_BREAK_S * rate as f32) as usize;
    let tail = (SQUELCH_TAIL_S * rate as f32) as usize;
    let floor = params.noise_level();
    let d = params.distance.clamp(0.0, 1.0);
    // The voice weakens toward the edge of range; the noise does not.
    let gain = 1.0 - 0.45 * d;
    let mut band = Band::new(r, params.radio.low_cut_hz, params.radio.high_cut_hz);
    let mut noise_band = Band::new(r, 200.0, params.radio.high_cut_hz + 400.0);
    let seed = params.seed;
    let body = samples.len();
    let mut out = Vec::with_capacity(lead + body + tail);
    // Carrier click then the squelch burst: a short decaying thump and a noise burst.
    for i in 0..lead {
        let t = i as f32 / lead.max(1) as f32;
        let click = if i < (rate as usize / 1000) * 2 {
            0.22 * (1.0 - i as f32 / ((rate as usize / 1000) * 2) as f32)
        } else {
            0.0
        };
        out.push(noise_band.run(noise(i, seed)) * 0.16 * (1.0 - t) + click);
    }
    for (i, value) in samples.iter().enumerate() {
        let input = if value.is_finite() { *value } else { 0.0 };
        let voice = soft_clip(band.run(input) * gain, params.radio.drive);
        // Distant stations crackle: the noise is gated in and out of short bursts.
        let burst = if d > 0.5 {
            let block = noise(i / (rate as usize / 40).max(1), seed ^ 0x55) * 0.5 + 0.5;
            1.0 + (d - 0.5) * 2.0 * block
        } else {
            1.0
        };
        let hiss = noise_band.run(noise(i + lead, seed ^ 1)) * floor * burst * 2.0;
        out.push((voice + hiss).clamp(-1.0, 1.0));
    }
    // Squelch tail: the hiss fades, then a closing click.
    for i in 0..tail {
        let t = i as f32 / tail.max(1) as f32;
        let fade = (1.0 - t).powi(2);
        let click = if i + (rate as usize / 1000) * 2 >= tail {
            0.15
        } else {
            0.0
        };
        out.push(
            (noise_band.run(noise(i + lead + body, seed ^ 2)) * 0.14 * fade + click * (1.0 - fade))
                .clamp(-1.0, 1.0),
        );
    }
    *samples = out;
}

/// Two carriers keyed together: a heterodyne squeal (two tones a few tens of hertz apart beating) over
/// garbled static, in place of any readable voice. Capped at four seconds. `duration_s` is the length
/// of the overlap.
pub fn blocked_garble(duration_s: f32, rate: u32, seed: u32) -> Vec<f32> {
    let rate = rate.max(8000);
    let len = (duration_s.clamp(0.4, 4.0) * rate as f32) as usize;
    let r = f64::from(rate);
    let beat = 63.0 + (seed % 40) as f64;
    let base = 1180.0 + (seed % 7) as f64 * 40.0;
    let mut band = Band::new(r, 300.0, 3000.0);
    let mut out = Vec::with_capacity(len + (SQUELCH_TAIL_S * rate as f32) as usize);
    for i in 0..len {
        let t = i as f64 / r;
        let a = libm::sin(core::f64::consts::TAU * base * t);
        let b = libm::sin(core::f64::consts::TAU * (base + beat) * t);
        // Sum of two near tones: amplitude swells and collapses at the beat rate.
        let squeal = ((a + b) * 0.5) as f32;
        // Syllabic chop standing in for two voices being cut up by each other.
        let chop = 0.55 + 0.45 * noise(i / (rate as usize / 12).max(1), seed ^ 7).abs();
        let envelope = ((i as f32 / (0.01 * rate as f32)).min(1.0))
            * (((len - i) as f32) / (0.02 * rate as f32)).min(1.0);
        let hiss = noise(i, seed ^ 3) * 0.08;
        out.push((band.run(squeal * 0.4 * chop + hiss) * 1.4 * envelope).clamp(-1.0, 1.0));
    }
    let tail = (SQUELCH_TAIL_S * rate as f32) as usize;
    for i in 0..tail {
        let fade = (1.0 - i as f32 / tail as f32).powi(2);
        out.push(noise(i + len, seed ^ 4) * 0.12 * fade);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use flightcraft_control::controller_voice;
    fn rms(v: &[f32]) -> f64 {
        libm::sqrt(v.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / v.len().max(1) as f64)
    }
    fn tone(hz: f64, rate: u32, seconds: f64) -> Vec<f32> {
        (0..(f64::from(rate) * seconds) as usize)
            .map(|i| (libm::sin(i as f64 / f64::from(rate) * core::f64::consts::TAU * hz) * 0.1) as f32)
            .collect()
    }
    fn params(distance: f32) -> RadioParams {
        RadioParams {
            radio: controller_voice(0).radio,
            distance,
            seed: 9,
        }
    }
    #[test]
    fn the_radio_is_band_limited_and_adds_squelch_lead_and_tail() {
        let rate = 22050;
        let mut low = tone(50.0, rate, 1.0);
        let mut mid = tone(1000.0, rate, 1.0);
        let mut high = tone(9000.0, rate, 1.0);
        for v in [&mut low, &mut mid, &mut high] {
            radio_effect(v, rate, &params(0.0));
            assert!(v.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        }
        assert!(rms(&mid) > rms(&low) * 2.0);
        assert!(rms(&mid) > rms(&high) * 1.7);
        let lead = (SQUELCH_BREAK_S * rate as f32) as usize;
        let tail = (SQUELCH_TAIL_S * rate as f32) as usize;
        assert_eq!(mid.len(), rate as usize + lead + tail);
        // The squelch break is audible before the voice, and the carrier click ends the tail.
        assert!(mid[..lead].iter().any(|s| s.abs() > 0.05));
        assert!(mid[mid.len() - 20..].iter().any(|s| s.abs() > 0.1));
    }
    #[test]
    fn noise_rises_with_distance_and_is_deterministic() {
        let rate = 22050;
        let silence = |d: f32| {
            let mut v = vec![0.0; rate as usize];
            radio_effect(&mut v, rate, &params(d));
            rms(&v[2000..rate as usize - 2000])
        };
        let (near, mid, far) = (silence(0.0), silence(0.5), silence(1.0));
        assert!(near < mid && mid < far * 0.9, "{near} {mid} {far}");
        assert!(far > near * 2.5);
        let mut a = tone(800.0, rate, 0.3);
        let mut b = a.clone();
        radio_effect(&mut a, rate, &params(0.7));
        radio_effect(&mut b, rate, &params(0.7));
        assert_eq!(a, b);
        // Distant stations are also weaker.
        let (mut close, mut distant) = (tone(800.0, rate, 0.5), tone(800.0, rate, 0.5));
        radio_effect(&mut close, rate, &params(0.0));
        radio_effect(&mut distant, rate, &params(1.0));
        let n = close.len().min(distant.len());
        assert!(rms(&distant[3000..n - 3000]) != rms(&close[3000..n - 3000]));
    }
    #[test]
    fn overlapping_carriers_squeal_at_the_beat_frequency() {
        let rate = 22050;
        let garble = blocked_garble(1.5, rate, 3);
        assert!(garble.len() > rate as usize);
        assert!(garble.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
        assert!(rms(&garble) > 0.03);
        // Energy sits inside the voice band, not at DC or near Nyquist.
        let dc: f64 = garble.iter().map(|s| f64::from(*s)).sum::<f64>() / garble.len() as f64;
        assert!(dc.abs() < 0.01);
        assert_eq!(garble, blocked_garble(1.5, rate, 3));
        assert_ne!(garble, blocked_garble(1.5, rate, 4));
        // The overlap is capped.
        assert!(blocked_garble(60.0, rate, 3).len() < (4.2 * rate as f32) as usize);
    }
}
