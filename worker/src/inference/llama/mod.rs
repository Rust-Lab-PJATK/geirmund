mod internal;

use crate::inference::llama::internal::LlamaInternals;
use crate::inference::{TextModel, TextModelConfig, TextModelFiles};
use anyhow::{Error as E, Result};
use candle_core::Tensor;
use candle_transformers::models::llama::LlamaEosToks;
use derive_more::Debug as DMDebug;
use std::sync::Arc;
use std::time::Instant;

#[derive(DMDebug)]
pub struct LLama {
    pub text_model_config: Arc<TextModelConfig>,
    pub text_model_files: Arc<TextModelFiles>,
    pub llama_internals: LlamaInternals,
}

impl TextModel for LLama {
    fn new(
        text_model_config: Arc<TextModelConfig>,
        text_model_files: Arc<TextModelFiles>,
    ) -> Result<Self> {
        let llama_internals = LlamaInternals::new(&text_model_config, &text_model_files)?;
        Ok(Self {
            text_model_config,
            text_model_files,
            llama_internals,
        })
    }

    #[tracing::instrument(skip_all)]
    fn generate(&mut self, prompt: String) -> Result<String> {
        let stared_at = Instant::now();
        tracing::debug!("Generating...");

        let mut tokens = self
            .llama_internals
            .tokenizer
            .encode(prompt, true)
            .map_err(E::msg)?
            .get_ids()
            .to_vec();

        let mut idx_pos = 0;
        for idx in 0..self.text_model_config.max_tokens {
            let (context_size, context_index) =
                if self.llama_internals.cache.use_kv_cache && idx > 0 {
                    (1, idx_pos)
                } else {
                    (tokens.len(), 0)
                };

            let ctxt = &tokens[tokens.len().saturating_sub(context_size)..];
            let input = Tensor::new(ctxt, &self.text_model_config.device)?.unsqueeze(0)?;
            let logits = self
                .llama_internals
                .inner_model
                .forward(&input, context_index, &mut self.llama_internals.cache)?
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

            let next_token = self.llama_internals.logits_processor.sample(&logits)?;
            tokens.push(next_token);

            match self.llama_internals.inner_config.eos_token_id {
                Some(LlamaEosToks::Single(eos_tok_id)) if next_token == eos_tok_id => {
                    break;
                }
                Some(LlamaEosToks::Multiple(ref eos_ids)) if eos_ids.contains(&next_token) => {
                    break;
                }
                _ => (),
            }
        }

        tracing::debug!("Generated in {}ms", stared_at.elapsed().as_millis());
        Ok(self
            .llama_internals
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
