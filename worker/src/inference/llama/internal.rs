use std::sync::OnceLock;
use crate::inference::{Device, Model, TextModelConfig, TextModelFiles};
use anyhow::Error as E;
use candle_nn::VarBuilder;
use candle_transformers::generation::{LogitsProcessor, Sampling};
use candle_transformers::models::llama::{
    Cache, Config as CandleConfig, Llama as CandleLlama, LlamaConfig,
};
use derive_more::Debug as DMDebug;
use std::time::Instant;
use ort::execution_providers::{CPUExecutionProvider, CUDAExecutionProvider};
use ort::session::builder::GraphOptimizationLevel;
use tokenizers::Tokenizer;

#[derive(DMDebug)]
pub struct LlamaInternals {
    pub model: Model,
    pub tokenizer: Tokenizer,
}

impl LlamaInternals {
    #[tracing::instrument(skip_all)]
    pub fn new(
        text_model_config: &TextModelConfig,
        text_model_files: &TextModelFiles,
    ) -> anyhow::Result<LlamaInternals> {
        tracing::debug!("Loading Llama...");
        let start_time = Instant::now();
        
        let execution_provider = match text_model_config.device {
            Device::CPU => CPUExecutionProvider::default().build(),
            Device::CUDA(id) => CUDAExecutionProvider::default().with_device_id(id as i32).build(),
        };
        ort::init()
            .with_name("GPT-2")
            .with_execution_providers([execution_provider])
            .commit()?;

        let model = Model::builder()?
            .with_optimization_level(GraphOptimizationLevel::Level1)?
            .with_intra_threads(1)?
            .commit_from_file(&text_model_files.weight_filenames.first().unwrap())?;
        
        let tokenizer = Tokenizer::from_file(&text_model_files.tokenizer_filename)
            .map_err(E::msg)?;

        tracing::debug!("Loaded in {}ms", start_time.elapsed().as_millis());
        Ok(LlamaInternals {
            model,
            tokenizer,
        })
    }
}
