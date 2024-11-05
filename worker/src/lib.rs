use anyhow::Result;
use candle_core::{DType, Device};

pub mod llama;

pub trait TextModel {
    fn load(&mut self) -> Result<()>;
    fn generate(&mut self, prompt: String) -> Result<String>;
    fn config(&self) -> &TextModelConfig;
    fn filenames(&self) -> &TextModelFiles;
}

#[derive(Debug)]
pub struct TextModelConfig {
    pub device: Device,
    pub data_type: DType,
    pub max_tokens: usize,
    pub temperature: f64,
    pub top_p: f64,
    pub top_k: usize,
    pub repeat_penalty: f32,
    pub repeat_last_n: usize,
}

#[derive(Debug)]
pub struct TextModelConfigBuilder {
    text_model_config: TextModelConfig,
}

#[derive(Debug)]
pub struct TextModelFiles {
    pub inner_config_filename: &'static str,
    pub tokenizer_filename: &'static str,
    pub weight_filenames: &'static [&'static str],
}

impl TextModelConfig {
    pub fn default_builder() -> TextModelConfigBuilder {
        TextModelConfigBuilder::default()
    }
}

impl TextModelConfigBuilder {
    pub fn with_device(mut self, device: Device) -> Self {
        self.text_model_config.device = device;
        self
    }

    pub fn with_data_type(mut self, data_type: DType) -> Self {
        self.text_model_config.data_type = data_type;
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: usize) -> Self {
        self.text_model_config.max_tokens = max_tokens;
        self
    }

    pub fn with_temperature(mut self, temperature: f64) -> Self {
        self.text_model_config.temperature = temperature;
        self
    }

    pub fn with_top_p(mut self, top_p: f64) -> Self {
        self.text_model_config.top_p = top_p;
        self
    }

    pub fn with_top_k(mut self, top_k: usize) -> Self {
        self.text_model_config.top_k = top_k;
        self
    }

    pub fn with_repeat_penalty(mut self, repeat_penalty: f32) -> Self {
        self.text_model_config.repeat_penalty = repeat_penalty;
        self
    }

    pub fn with_repeat_last_n(mut self, repeat_last_n: usize) -> Self {
        self.text_model_config.repeat_last_n = repeat_last_n;
        self
    }

    pub fn build(self) -> TextModelConfig {
        self.text_model_config
    }
}

impl Default for TextModelConfigBuilder {
    fn default() -> Self {
        Self {
            text_model_config: TextModelConfig {
                device: Device::Cpu,
                data_type: DType::F16,
                max_tokens: 512,
                temperature: 0.8,
                top_p: 0.9,
                top_k: 40,
                repeat_penalty: 1.1,
                repeat_last_n: 128,
            },
        }
    }
}

impl TextModelFiles {
    pub fn new(
        inner_config_filename: &'static str,
        tokenizer_filename: &'static str,
        weight_filenames: &'static [&'static str],
    ) -> Self {
        Self {
            inner_config_filename,
            tokenizer_filename,
            weight_filenames,
        }
    }
}
