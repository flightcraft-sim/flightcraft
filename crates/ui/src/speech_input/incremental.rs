//! Incremental Whisper text decoder. Self-attention appends K/V once per token;
//! cross-attention projects the audio once per utterance. Original implementation
//! of the published Whisper transformer equations using safe Candle tensor ops.
use candle_core::{D, Result, Tensor};
use candle_nn::{
    Embedding, LayerNorm, Linear, Module, VarBuilder, embedding, layer_norm, linear, linear_no_bias,
};
use candle_transformers::models::whisper::Config;

struct Attention {
    query: Linear,
    key: Linear,
    value: Linear,
    output: Linear,
    heads: usize,
    cache: Option<(Tensor, Tensor)>,
}
impl Attention {
    fn load(width: usize, heads: usize, vb: VarBuilder<'_>) -> Result<Self> {
        Ok(Self {
            query: linear(width, width, vb.pp("q_proj"))?,
            key: linear_no_bias(width, width, vb.pp("k_proj"))?,
            value: linear(width, width, vb.pp("v_proj"))?,
            output: linear(width, width, vb.pp("out_proj"))?,
            heads,
            cache: None,
        })
    }
    fn split(&self, x: &Tensor) -> Result<Tensor> {
        let (batch, sequence, width) = x.dims3()?;
        x.reshape((batch, sequence, self.heads, width / self.heads))?
            .transpose(1, 2)?
            .contiguous()
    }
    fn attend(&mut self, input: &Tensor, audio: Option<&Tensor>, mask: Option<&Tensor>) -> Result<Tensor> {
        let query = self.split(&self.query.forward(input)?)?;
        if audio.is_none() || self.cache.is_none() {
            let source = audio.unwrap_or(input);
            let mut key = self.split(&self.key.forward(source)?)?;
            let mut value = self.split(&self.value.forward(source)?)?;
            if audio.is_none()
                && let Some((old_key, old_value)) = &self.cache
            {
                key = Tensor::cat(&[old_key, &key], 2)?;
                value = Tensor::cat(&[old_value, &value], 2)?;
            }
            self.cache = Some((key, value));
        }
        let (key, value) = self
            .cache
            .as_ref()
            .ok_or_else(|| candle_core::Error::Msg("Missing Whisper attention cache".into()))?;
        let scale = (query.dim(D::Minus1)? as f64).powf(-0.25);
        let mut scores = (query * scale)?.matmul(&(key.transpose(2, 3)? * scale)?)?;
        if let Some(mask) = mask {
            scores = scores.broadcast_add(mask)?;
        }
        let attended = candle_nn::ops::softmax_last_dim(&scores)?
            .matmul(value)?
            .transpose(1, 2)?
            .flatten_from(2)?;
        self.output.forward(&attended)
    }
}
struct Block {
    self_attention: Attention,
    self_norm: LayerNorm,
    cross_attention: Attention,
    cross_norm: LayerNorm,
    feed_norm: LayerNorm,
    expand: Linear,
    contract: Linear,
}
impl Block {
    fn load(c: &Config, vb: VarBuilder<'_>) -> Result<Self> {
        Ok(Self {
            self_attention: Attention::load(c.d_model, c.decoder_attention_heads, vb.pp("self_attn"))?,
            self_norm: layer_norm(c.d_model, 1e-5, vb.pp("self_attn_layer_norm"))?,
            cross_attention: Attention::load(c.d_model, c.decoder_attention_heads, vb.pp("encoder_attn"))?,
            cross_norm: layer_norm(c.d_model, 1e-5, vb.pp("encoder_attn_layer_norm"))?,
            feed_norm: layer_norm(c.d_model, 1e-5, vb.pp("final_layer_norm"))?,
            expand: linear(c.d_model, 4 * c.d_model, vb.pp("fc1"))?,
            contract: linear(4 * c.d_model, c.d_model, vb.pp("fc2"))?,
        })
    }
    fn step(&mut self, x: &Tensor, audio: &Tensor, mask: Option<&Tensor>) -> Result<Tensor> {
        let x = (x + self
            .self_attention
            .attend(&self.self_norm.forward(x)?, None, mask)?)?;
        let x = (&x
            + self
                .cross_attention
                .attend(&self.cross_norm.forward(&x)?, Some(audio), None)?)?;
        let feed = self
            .contract
            .forward(&self.expand.forward(&self.feed_norm.forward(&x)?)?.gelu()?)?;
        x + feed
    }
}
pub(super) struct Decoder {
    embedding: Embedding,
    positions: Tensor,
    norm: LayerNorm,
    blocks: Vec<Block>,
    offset: usize,
}
impl Decoder {
    pub fn load(config: &Config, vb: VarBuilder<'_>) -> Result<Self> {
        Ok(Self {
            embedding: embedding(config.vocab_size, config.d_model, vb.pp("embed_tokens"))?,
            positions: vb.get(
                (config.max_target_positions, config.d_model),
                "embed_positions.weight",
            )?,
            norm: layer_norm(config.d_model, 1e-5, vb.pp("layer_norm"))?,
            blocks: (0..config.decoder_layers)
                .map(|i| Block::load(config, vb.pp(format!("layers.{i}"))))
                .collect::<Result<_>>()?,
            offset: 0,
        })
    }
    pub fn reset(&mut self) {
        self.offset = 0;
        for block in &mut self.blocks {
            block.self_attention.cache = None;
            block.cross_attention.cache = None;
        }
    }
    pub fn logits(&mut self, tokens: &[u32], audio: &Tensor) -> Result<Tensor> {
        let length = tokens.len();
        if length == 0 {
            candle_core::bail!("Empty Whisper token input");
        }
        let input = Tensor::new(tokens, audio.device())?.unsqueeze(0)?;
        let mut x = self
            .embedding
            .forward(&input)?
            .broadcast_add(&self.positions.narrow(0, self.offset, length)?)?;
        // Only the initial prompt needs a causal mask. Subsequent single-token
        // queries can attend to every cached key, without re-running the prefix.
        let mask = if length > 1 {
            let total = self.offset + length;
            let offset = self.offset;
            let values: Vec<f32> = (0..length)
                .flat_map(|q| (0..total).map(move |k| if k > offset + q { f32::NEG_INFINITY } else { 0. }))
                .collect();
            Some(Tensor::from_vec(values, (length, total), audio.device())?)
        } else {
            None
        };
        for block in &mut self.blocks {
            x = block.step(&x, audio, mask.as_ref())?;
        }
        self.offset += length;
        let last = self.norm.forward(&x.narrow(1, length - 1, 1)?)?;
        last.matmul(&self.embedding.embeddings().t()?.unsqueeze(0)?)?
            .squeeze(0)?
            .squeeze(0)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use candle_core::{DType, Device, Shape};
    struct FixtureWeights;
    impl candle_nn::var_builder::SimpleBackend for FixtureWeights {
        fn get(
            &self,
            shape: Shape,
            name: &str,
            _: candle_nn::Init,
            dtype: DType,
            device: &Device,
        ) -> Result<Tensor> {
            // Nonzero deterministic weights exercise attention, offsets, and residuals.
            let seed = name.bytes().map(usize::from).sum::<usize>();
            let values: Vec<f32> = (0..shape.elem_count())
                .map(|i| {
                    if name.contains("norm") && name.ends_with("weight") {
                        1.
                    } else {
                        ((i + seed) as f32 * 0.731).sin() * 0.15
                    }
                })
                .collect();
            Tensor::from_vec(values, shape, device)?.to_dtype(dtype)
        }
        fn get_unchecked(&self, name: &str, _: DType, _: &Device) -> Result<Tensor> {
            candle_core::bail!("Fixture tensor {name} requires an explicit shape")
        }
        fn contains_tensor(&self, _: &str) -> bool {
            true
        }
    }
    #[test]
    fn incremental_logits_match_full_decoder_and_reset_between_utterances() {
        let config = Config {
            num_mel_bins: 4,
            max_source_positions: 10,
            d_model: 8,
            encoder_attention_heads: 2,
            encoder_layers: 1,
            vocab_size: 32,
            max_target_positions: 24,
            decoder_attention_heads: 2,
            decoder_layers: 2,
            suppress_tokens: vec![],
        };
        let vb = VarBuilder::from_backend(Box::new(FixtureWeights), DType::F32, Device::Cpu);
        let mut reference =
            candle_transformers::models::whisper::model::Whisper::load(&vb, config.clone()).unwrap();
        let mut cached = Decoder::load(&config, vb.pp("model.decoder")).unwrap();
        for shift in [0.1, 1.3] {
            cached.reset();
            let audio = Tensor::from_vec(
                (0..56).map(|i| (i as f32 + shift).sin()).collect::<Vec<_>>(),
                (1, 7, 8),
                &Device::Cpu,
            )
            .unwrap();
            let mut tokens = vec![1, 4, 9];
            for step in 0..5 {
                let input = if step == 0 {
                    tokens.as_slice()
                } else {
                    tokens.get(tokens.len() - 1..).unwrap()
                };
                let got = cached.logits(input, &audio).unwrap().to_vec1::<f32>().unwrap();
                let all = Tensor::new(tokens.as_slice(), &Device::Cpu)
                    .unwrap()
                    .unsqueeze(0)
                    .unwrap();
                let hidden = reference.decoder.forward(&all, &audio, true).unwrap();
                let last = hidden.narrow(1, tokens.len() - 1, 1).unwrap();
                let expected = reference
                    .decoder
                    .final_linear(&last)
                    .unwrap()
                    .flatten_all()
                    .unwrap()
                    .to_vec1::<f32>()
                    .unwrap();
                for (a, b) in got.iter().zip(expected) {
                    assert!((*a - b).abs() < 0.0001, "step {step}: {a} != {b}");
                }
                tokens.push((step + 12) as u32);
            }
        }
    }
}
