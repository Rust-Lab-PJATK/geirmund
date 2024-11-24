use crate::inference::{TextModelConfig, TextModelFiles};
use anyhow::Error as E;
use candle_nn::VarBuilder;
use candle_transformers::generation::{LogitsProcessor, Sampling};
use candle_transformers::models::llama::{
    Cache, Config as CandleConfig, Llama as CandleLlama, LlamaConfig,
};
use derive_more::Debug as DMDebug;
use std::time::Instant;
use tokenizers::Tokenizer;

#[derive(DMDebug)]
pub struct LlamaInternals {
    pub inner_config: CandleConfig,
    pub inner_model: CandleLlama,
    pub cache: Cache,
    pub tokenizer: Tokenizer,
    #[debug(skip)]
    pub logits_processor: LogitsProcessor,
}

impl LlamaInternals {
    fn load_inner_config(filename: &str) -> anyhow::Result<CandleConfig> {
        let config = std::fs::read_to_string(filename)?;
        let config: LlamaConfig = serde_json::from_str(&config)?;
        Ok(config.into_config(false))
    }

    fn make_logits_processor(text_model_config: &TextModelConfig) -> LogitsProcessor {
        let sampling = if text_model_config.temperature <= 0. {
            Sampling::ArgMax
        } else {
            Sampling::TopKThenTopP {
                k: text_model_config.top_k,
                p: text_model_config.top_p,
                temperature: text_model_config.temperature,
            }
        };
        LogitsProcessor::from_sampling(1337, sampling)
    }

    #[tracing::instrument(skip_all)]
    pub fn new(
        text_model_config: &TextModelConfig,
        text_model_files: &TextModelFiles,
    ) -> anyhow::Result<LlamaInternals> {
        tracing::debug!("Loading Llama...");
        let start_time = Instant::now();

        let inner_config = Self::load_inner_config(&text_model_files.inner_config_filename)?;
        let vb = unsafe {
            VarBuilder::from_mmaped_safetensors(
                &text_model_files.weight_filenames,
                text_model_config.data_type,
                &text_model_config.device,
            )?
        };
        let inner_model = CandleLlama::load(vb, &inner_config)?;
        let cache = Cache::new(
            false,
            text_model_config.data_type,
            &inner_config,
            &text_model_config.device,
        )?;
        let tokenizer =
            Tokenizer::from_file(&text_model_files.tokenizer_filename).map_err(E::msg)?;
        let logits_processor = Self::make_logits_processor(text_model_config);

        tracing::debug!("Loaded in {}ms", start_time.elapsed().as_millis());

        Ok(LlamaInternals {
            inner_config,
            inner_model,
            cache,
            tokenizer,
            logits_processor,
        })
    }
}
