use anyhow::Result;

pub mod llama;

pub trait TextModel {
    fn generate(&mut self, prompt: String) -> Result<String>;
}