use crate::{TextModel, TextModelConfig, TextModelFiles};
use anyhow::{bail, Error as E, Result};
use candle_core::Tensor;
use candle_nn::VarBuilder;
use candle_transformers::generation::{LogitsProcessor, Sampling};
use candle_transformers::models::llama::{
    Cache, Config as CandleConfig, Llama as CandleLlama, LlamaConfig, LlamaEosToks,
};
use derive_more::Debug as DMDebug;
use std::time::Instant;
use tokenizers::Tokenizer;

#[derive(DMDebug)]
pub struct LLama {
    pub text_model_config: TextModelConfig,
    pub text_model_files: TextModelFiles,
    pub llama_internals: Option<LlamaInternals>,
}

#[derive(DMDebug)]
pub struct LlamaInternals {
    pub inner_config: CandleConfig,
    pub inner_model: CandleLlama,
    pub cache: Cache,
    pub tokenizer: Tokenizer,
    #[debug(skip)]
    pub logits_processor: LogitsProcessor,
}

impl LLama {
    pub fn new(text_model_config: TextModelConfig, text_model_files: TextModelFiles) -> Self {
        Self {
            text_model_config,
            text_model_files,
            llama_internals: None,
        }
    }

    fn load_inner_config(filename: &str) -> Result<CandleConfig> {
        let config = std::fs::read_to_string(filename)?;
        let config: LlamaConfig = serde_json::from_str(&config)?;
        Ok(config.into_config(false))
    }

    fn make_logits_processor(&self) -> LogitsProcessor {
        let sampling = if self.text_model_config.temperature <= 0. {
            Sampling::ArgMax
        } else {
            Sampling::TopKThenTopP {
                k: self.text_model_config.top_k,
                p: self.text_model_config.top_p,
                temperature: self.text_model_config.temperature,
            }
        };
        LogitsProcessor::from_sampling(1337, sampling)
    }
}

impl TextModel for LLama {
    fn load(&mut self) -> Result<()> {
        println!("Loading Llama...");
        let start_time = Instant::now();

        let inner_config = Self::load_inner_config(self.text_model_files.inner_config_filename)?;
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(
                self.text_model_files.weight_filenames,
                self.text_model_config.data_type,
                &self.text_model_config.device,
            )?
        };
        let inner_model = CandleLlama::load(vb, &inner_config)?;
        let cache = Cache::new(
            true,
            self.text_model_config.data_type,
            &inner_config,
            &self.text_model_config.device,
        )?;
        let tokenizer =
            Tokenizer::from_file(self.text_model_files.tokenizer_filename).map_err(E::msg)?;
        let logits_processor = self.make_logits_processor();

        println!("Loaded in {}ms", start_time.elapsed().as_millis());
        self.llama_internals = Some(LlamaInternals {
            inner_config,
            inner_model,
            cache,
            tokenizer,
            logits_processor,
        });
        Ok(())
    }

    fn generate(&mut self, prompt: String) -> Result<String> {
        let llama_internals = if let Some(ref mut li) = self.llama_internals {
            li
        } else {
            bail!("LLama must be loaded before generating")
        };

        let mut tokens = llama_internals
            .tokenizer
            .encode(prompt, true)
            .map_err(E::msg)?
            .get_ids()
            .to_vec();

        let mut idx_pos = 0;
        for idx in 0..self.text_model_config.max_tokens {
            let (context_size, context_index) = if llama_internals.cache.use_kv_cache && idx > 0 {
                (1, idx_pos)
            } else {
                (tokens.len(), 0)
            };

            let ctxt = &tokens[tokens.len().saturating_sub(context_size)..];
            let input = Tensor::new(ctxt, &self.text_model_config.device)?.unsqueeze(0)?;
            let logits = llama_internals
                .inner_model
                .forward(&input, context_index, &mut llama_internals.cache)?
                .squeeze(0)?;
            let logits = if self.text_model_config.repeat_penalty == 1. {
                logits
            } else {
                let start_at = tokens
                    .len()
                    .saturating_sub(self.text_model_config.repeat_last_n);
                candle_transformers::utils::apply_repeat_penalty(
                    &logits,
                    self.text_model_config.repeat_penalty,
                    &tokens[start_at..],
                )?
            };
            idx_pos += ctxt.len();

            let next_token = llama_internals.logits_processor.sample(&logits)?;
            tokens.push(next_token);

            match llama_internals.inner_config.eos_token_id {
                Some(LlamaEosToks::Single(eos_tok_id)) if next_token == eos_tok_id => {
                    break;
                }
                Some(LlamaEosToks::Multiple(ref eos_ids)) if eos_ids.contains(&next_token) => {
                    break;
                }
                _ => (),
            }
        }

        Ok(llama_internals
            .tokenizer
            .decode(tokens.as_slice(), true)
            .map_err(E::msg)?)
    }

    fn config(&self) -> &TextModelConfig {
        &self.text_model_config
    }

    fn filenames(&self) -> &TextModelFiles {
        &self.text_model_files
    }
}
