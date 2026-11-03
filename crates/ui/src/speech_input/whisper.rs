//! Pure-Rust, CPU-only Candle Whisper. No model or tokenizer is embedded in the app.
use super::{
    models,
    pipeline::{AudioClip, Recognized, Recognizer},
};
use candle_core::{Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::whisper::{self, model::Whisper as Model};
use std::time::Instant;
use tokenizers::Tokenizer;
#[derive(Clone, Debug, Default, serde::Serialize)]
pub(super) struct StageTimings {
    pub resample_ms: f64,
    pub mel_ms: f64,
    pub encoder_ms: f64,
    pub decoder_ms: f64,
    pub tokens: usize,
    pub mel_frames: usize,
    pub inference_threads: usize,
}
pub struct Whisper {
    pub(super) timings: StageTimings,
    encoder: whisper::model::AudioEncoder,
    decoder: super::incremental::Decoder,
    pool: std::sync::Arc<rayon::ThreadPool>,
    pub(super) minimum_context_seconds: usize,
    permitted: Vec<bool>,
    config: whisper::Config,
    tokenizer: Tokenizer,
    frontend: super::frontend::Frontend,
    blank: u32,
    prefix: Vec<u32>,
    start_previous: u32,
    sot: u32,
    eot: u32,
    no_timestamps: u32,
    no_speech: u32,
}
impl Whisper {
    pub fn load(id: &str) -> Result<Self, String> {
        let root = models::verify(id)?;
        let config: whisper::Config =
            serde_json::from_slice(&std::fs::read(root.join("config.json")).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        let tokenizer = Tokenizer::from_file(root.join("tokenizer.json")).map_err(|e| e.to_string())?;
        let token = |name| {
            tokenizer
                .token_to_id(name)
                .ok_or_else(|| format!("Whisper tokenizer is missing {name}."))
        };
        let sot = token("<|startoftranscript|>")?;
        let start_previous = token("<|startofprev|>")?;
        let eot = token("<|endoftext|>")?;
        let no_timestamps = token("<|notimestamps|>")?;
        let no_speech = tokenizer
            .token_to_id("<|nospeech|>")
            .or_else(|| tokenizer.token_to_id("<|nocaptions|>"))
            .ok_or("Whisper tokenizer has no silence token.")?;
        let blank = tokenizer
            .encode(" ", false)
            .map_err(|e| e.to_string())?
            .get_ids()
            .first()
            .copied()
            .ok_or("Whisper tokenizer has no space token.")?;
        let weights = std::fs::read(root.join("model.safetensors")).map_err(|e| e.to_string())?;
        let vb = VarBuilder::from_buffered_safetensors(weights, whisper::DTYPE, &Device::Cpu)
            .map_err(|e| e.to_string())?;
        let model = Model::load(&vb, config.clone()).map_err(|e| e.to_string())?;
        let decoder =
            super::incremental::Decoder::load(&config, vb.pp("model.decoder")).map_err(|e| e.to_string())?;
        let available = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
        let cap = available.saturating_sub(2).max(1);
        let requested = std::env::var("RAYON_NUM_THREADS")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(cap);
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(requested.min(cap))
            .thread_name(|i| format!("speech-cpu-{i}"))
            .build()
            .map_err(|e| e.to_string())?;
        let mut permitted = vec![false; config.vocab_size];
        for (id, allow) in permitted.iter_mut().enumerate() {
            *allow = id <= eot as usize && !config.suppress_tokens.contains(&(id as u32));
        }
        Ok(Self {
            encoder: model.encoder,
            decoder,
            pool: std::sync::Arc::new(pool),
            minimum_context_seconds: 10,
            permitted,
            timings: StageTimings::default(),
            tokenizer,
            frontend: super::frontend::Frontend::new(config.num_mel_bins),
            blank,
            prefix: Vec::new(),
            start_previous,
            config,
            sot,
            eot,
            no_timestamps,
            no_speech,
        })
    }
    /// Previous-context prompt from live, heard radio state. It biases decoding;
    /// the acoustic result still passes the grammar and independent grading.
    pub fn set_context(&mut self, names: &str) -> Result<(), String> {
        self.prefix.clear();
        if !names.is_empty() {
            let encoded = self.tokenizer.encode(names, false).map_err(|e| e.to_string())?;
            self.prefix.push(self.start_previous);
            self.prefix.extend(encoded.get_ids().iter().copied().take(96));
        }
        Ok(())
    }
}
impl Recognizer for Whisper {
    fn set_context(&mut self, context: &str) -> Result<(), String> {
        Whisper::set_context(self, context)
    }
    fn recognize(&mut self, clip: &AudioClip) -> Result<Recognized, String> {
        let pool = self.pool.clone();
        pool.install(|| self.recognize_inner(clip))
    }
}
impl Whisper {
    fn recognize_inner(&mut self, clip: &AudioClip) -> Result<Recognized, String> {
        let start = Instant::now();
        self.timings = StageTimings {
            inference_threads: rayon::current_num_threads(),
            ..StageTimings::default()
        };
        let pcm = clip.mono_16k()?;
        self.timings.resample_ms = start.elapsed().as_secs_f64() * 1000.;
        let rms = (pcm.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / pcm.len().max(1) as f64).sqrt();
        if rms < 0.003 {
            return Ok(Recognized {
                text: String::new(),
                confidence: 0.,
                latency_ms: start.elapsed().as_millis() as u64,
                alternatives: Vec::new(),
            });
        }
        // Whole seconds, at most the trained 30-second context. The encoder
        // slices its sinusoidal positional embedding to the matching length.
        let frames = pcm
            .len()
            .div_ceil(160)
            .div_ceil(100)
            .saturating_mul(100)
            .max(self.minimum_context_seconds * 100)
            .min(whisper::N_FRAMES);
        self.timings.mel_frames = frames;
        let stage = Instant::now();
        let mel = self.frontend.mel(&pcm, frames);
        self.timings.mel_ms = stage.elapsed().as_secs_f64() * 1000.;
        let stage = Instant::now();
        let mel = Tensor::from_vec(mel, (1, self.config.num_mel_bins, frames), &Device::Cpu)
            .map_err(|e| e.to_string())?;
        let features = self.encoder.forward(&mel, true).map_err(|e| e.to_string())?;
        self.timings.encoder_ms = stage.elapsed().as_secs_f64() * 1000.;
        let stage = Instant::now();
        self.decoder.reset();
        let mut input = self.prefix.clone();
        input.extend([self.sot, self.no_timestamps]);
        let mut tokens = Vec::new();
        let mut score = 0f64;
        let mut no_speech = 0.;
        let mut ended = false;
        for position in 0..64.min(self.config.max_target_positions.saturating_sub(input.len())) {
            let logits = self
                .decoder
                .logits(&input, &features)
                .and_then(|t| t.to_vec1::<f32>())
                .map_err(|e| e.to_string())?;
            let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            if position == 0 {
                let total = logits.iter().map(|v| (*v - max).exp()).sum::<f32>();
                no_speech = logits
                    .get(self.no_speech as usize)
                    .map_or(1., |v| (*v - max).exp() / total);
            }
            let allowed = |id: usize| {
                self.permitted.get(id).copied().unwrap_or(false)
                    && (position != 0 || (id as u32 != self.blank && id as u32 != self.eot))
            };
            let mut chosen = None;
            let mut denominator = 0f32;
            for (id, value) in logits
                .iter()
                .copied()
                .enumerate()
                .filter(|(id, v)| v.is_finite() && allowed(*id))
            {
                denominator += (value - max).exp();
                if chosen.is_none_or(|(_, best)| value > best) {
                    chosen = Some((id as u32, value));
                }
            }
            let (token, value) = chosen
                .filter(|_| denominator.is_finite() && denominator > 0.)
                .ok_or("Whisper could not decode this call. Say again.")?;
            score += f64::from(value - max - denominator.ln());
            if token == self.eot {
                ended = true;
                break;
            }
            tokens.push(token);
            input.clear();
            input.push(token);
        }
        self.timings.decoder_ms = stage.elapsed().as_secs_f64() * 1000.;
        self.timings.tokens = tokens.len();
        if !ended {
            return Err("Whisper did not finish this call within the radio token limit. Say again.".into());
        }
        let text = self.tokenizer.decode(&tokens, true).map_err(|e| e.to_string())?;
        let confidence = if tokens.is_empty() || no_speech > 0.6 {
            0.
        } else {
            (score / (tokens.len() + 1) as f64).exp() as f32
        };
        Ok(Recognized {
            text: text.trim().into(),
            confidence,
            latency_ms: start.elapsed().as_millis() as u64,
            alternatives: Vec::new(),
        })
    }
}
