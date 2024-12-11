mod internal;

use crate::inference::llama::internal::LlamaInternals;
use crate::inference::{TextModel, TextModelConfig, TextModelFiles};
use anyhow::{Error as E, Result};
use candle_core::Tensor;
use candle_transformers::models::llama::LlamaEosToks;
use derive_more::Debug as DMDebug;
use std::sync::Arc;
use std::time::Instant;
use ort::inputs;
use rand::Rng;

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

        let mut rng = rand::thread_rng();

        let tokens = self.llama_internals.tokenizer.encode(prompt, false)
            .map_err(E::msg)?;
        let mut tokens = Arc::new(
            tokens.get_ids().iter()
                .map(|i| *i as i64)
                .collect::<Vec<_>>()
                .into_boxed_slice()
        );

        for _ in 0..self.text_model_config.max_tokens {
            let input = (vec![1, 1, tokens.len() as i64], Arc::clone(&tokens));
            let outputs = self.llama_internals.model.run(inputs![input]?)?;
            let (dim, mut probabilities) = outputs["output1"].try_extract_raw_tensor()?;

            let (seq_len, vocab_size) = (dim[2] as usize, dim[3] as usize);
            probabilities = &probabilities[(seq_len - 1) * vocab_size..];

            let mut probabilities: Vec<(usize, f32)> = probabilities.iter().copied().enumerate().collect();
            probabilities.sort_unstable_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Less));

            let token = probabilities[rng.gen_range(0..=self.text_model_config.top_k)].0 as i64;

            let mut vec = tokens.to_vec();
            vec.push(token);
            *Arc::make_mut(&mut tokens) = vec.into_boxed_slice();
        }
    
        Ok(
            self.llama_internals.tokenizer.decode(
                &*tokens.iter().map(|i| *i as u32).collect::<Vec<_>>(), false
            ).map_err(E::msg)?,
        )
    }

    fn config(&self) -> &TextModelConfig {
        &self.text_model_config
    }

    fn filenames(&self) -> &TextModelFiles {
        &self.text_model_files
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;
    use crate::inference::llama::LLama;
    use crate::inference::{TextModel, TextModelConfig, TextModelFiles};

    #[test]
    fn run_gpt2() {
        let tmc = TextModelConfig::default_builder().build();
        let fmc = TextModelFiles::new("llama3v2-1b/config.json".to_string(), "llama3v2-1b/tokenizer.json".to_string(), vec!["llama3v2-1b/model.safetensors".to_string()]);
        let model = LLama::new(Arc::new(tmc), Arc::new(fmc)).unwrap();
    }
}
