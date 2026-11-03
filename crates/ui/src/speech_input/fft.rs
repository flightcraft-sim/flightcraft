//! Original mixed-radix FFT: 400 = 2^4 * 5^2, with reusable roots and scratch.
//! Safe Rust, no native FFT library and no per-frame planning or allocation.
#[derive(Clone, Copy, Default)]
pub(super) struct Complex {
    pub re: f32,
    pub im: f32,
}
pub(super) struct Fft {
    length: usize,
    radix: usize,
    roots: Vec<Complex>,
    child: Option<Box<Fft>>,
}
impl Fft {
    pub fn new(length: usize) -> Self {
        let radix = if length.is_multiple_of(2) { 2 } else { 5 };
        let roots = (0..length)
            .map(|i| {
                let angle = -std::f64::consts::TAU * i as f64 / length as f64;
                Complex {
                    re: angle.cos() as f32,
                    im: angle.sin() as f32,
                }
            })
            .collect();
        Self {
            length,
            radix,
            roots,
            child: if length > 1 {
                Some(Box::new(Self::new(length / radix)))
            } else {
                None
            },
        }
    }
    pub fn forward(&self, input: &[Complex], output: &mut [Complex], scratch: &mut [Complex]) {
        self.transform(input, 0, 1, output, scratch);
    }
    fn transform(
        &self,
        input: &[Complex],
        offset: usize,
        stride: usize,
        output: &mut [Complex],
        scratch: &mut [Complex],
    ) {
        let Some(child) = &self.child else {
            if let Some(value) = output.first_mut() {
                *value = input.get(offset).copied().unwrap_or_default();
            }
            return;
        };
        let width = self.length / self.radix;
        for (branch, part) in output.chunks_exact_mut(width).take(self.radix).enumerate() {
            child.transform(
                input,
                offset + branch * stride,
                stride * self.radix,
                part,
                scratch,
            );
        }
        for (i, value) in scratch.iter_mut().take(self.length).enumerate() {
            let mut sum = Complex::default();
            for branch in 0..self.radix {
                if let (Some(x), Some(w)) = (
                    output.get(branch * width + i % width),
                    self.roots.get(branch * i % self.length),
                ) {
                    sum.re += x.re * w.re - x.im * w.im;
                    sum.im += x.re * w.im + x.im * w.re;
                }
            }
            *value = sum;
        }
        for (value, transformed) in output.iter_mut().zip(scratch) {
            *value = *transformed;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fft_matches_direct_transform_for_whisper_window() {
        let input: Vec<_> = (0..400)
            .map(|i| Complex {
                re: (i as f32 * 0.137).sin(),
                im: 0.,
            })
            .collect();
        let mut output = vec![Complex::default(); 400];
        let mut scratch = output.clone();
        Fft::new(400).forward(&input, &mut output, &mut scratch);
        for (bin, got) in output.iter().enumerate() {
            let mut expected = Complex::default();
            for (i, x) in input.iter().enumerate() {
                let angle = -std::f64::consts::TAU * (bin * i) as f64 / 400.;
                expected.re += x.re * angle.cos() as f32;
                expected.im += x.re * angle.sin() as f32;
            }
            assert!(
                (got.re - expected.re).abs() < 0.001 && (got.im - expected.im).abs() < 0.001,
                "bin {bin}"
            );
        }
    }
}
