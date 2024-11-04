use candle_core::{CudaDevice, DType, Device, Tensor};
use candle_transformers::models::llama::{Cache, Config, Llama as CandleLlama, LlamaConfig, LlamaEosToks};
use tokenizers::Tokenizer;
use anyhow::{Result, Error as E};
use candle_core::backend::BackendDevice;
use candle_nn::VarBuilder;
use candle_transformers::generation::{LogitsProcessor, Sampling};
use crate::TextModel;

pub struct LLama {
    pub device: Device,
    pub data_type: DType,
    pub max_tokens: usize,

    pub config: Config,
    pub inner_model: CandleLlama,
    pub cache: Cache,
    pub tokenizer: Tokenizer,
}

impl LLama {
    pub fn new(
        config_file: &str,
        weight_files: &[&str],
        tokenizer_file: &str,
        data_type: DType,
        cuda_device_id: Option<usize>,
        max_tokens: Option<usize>,
    ) -> Result<Self > {
        let device = if let Some(id) = cuda_device_id {
            Device::Cuda(CudaDevice::new(id)?)
        } else {
            Device::Cpu
        };

        let config = Self::load_config(config_file)?;
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(weight_files, data_type, &device)?
        };
        let inner_model = CandleLlama::load(vb, &config)?;
        let cache = Cache::new(true, data_type, &config, &device)?;
        let tokenizer = Tokenizer::from_file(tokenizer_file).map_err(E::msg)?;

        let max_tokens = if let Some(mt) = max_tokens {
            mt
        } else {
            512
        };

        Ok(
            Self {
                config,
                device,
                data_type,
                max_tokens,
                inner_model,
                cache,
                tokenizer,
            }
        )
    }

    fn load_config(filename: &str) -> Result<Config> {
        let config = std::fs::read_to_string(filename)?;
        let config: LlamaConfig = serde_json::from_str(&config)?;
        Ok(config.into_config(false))
    }
}

impl Default for LLama {
    fn default() -> Self {
        Self::new(
            "worker/llama3v2-1b/config.json",
            &["worker/llama3v2-1b/model.safetensors"],
            "worker/llama3v2-1b/tokenizer.json",
            DType::F16,
            Some(0),
            None,
        ).expect("Failed to load Llama")
    }
}

impl TextModel for LLama {
    fn generate(&mut self, prompt: String) -> Result<String> {
        let mut tokens = self.tokenizer
            .encode(prompt, true)
            .map_err(E::msg)?
            .get_ids()
            .to_vec();
        let mut logits_processor = LogitsProcessor::from_sampling(1337, Sampling::ArgMax);

        let mut idx_pos = 0;
        for idx in 0..self.max_tokens {
            let (context_size, context_index) = if self.cache.use_kv_cache && idx > 0 {
                (1, idx_pos)
            } else {
                (tokens.len(), 0)
            };

            let ctxt = &tokens[tokens.len().saturating_sub(context_size)..];
            let input = Tensor::new(ctxt, &self.device)?.unsqueeze(0)?;
            let logits = self.inner_model.forward(&input, context_index, &mut self.cache)?.squeeze(0)?;
            idx_pos += 1;

            let next_token = logits_processor.sample(&logits)?;
            tokens.push(next_token);

            match self.config.eos_token_id {
                Some(LlamaEosToks::Single(eos_tok_id)) if next_token == eos_tok_id => {
                    break;
                }
                Some(LlamaEosToks::Multiple(ref eos_ids)) if eos_ids.contains(&next_token) => {
                    break;
                }
                _ => (),
            }
        }

        Ok(
            self.tokenizer
                .decode(tokens.as_slice(), true)
                .map_err(E::msg)?
        )
    }
}