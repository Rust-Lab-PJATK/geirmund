use anyhow::Result;
use std::time::Instant;
use worker::llama::LLama;
use worker::{TextModel, TextModelConfig, TextModelFiles};

fn main() -> Result<()> {
    let mut model = LLama::new(
        TextModelConfig::default_builder().build(),
        TextModelFiles::new(
            "worker/llama3v2-1b/config.json",
            "worker/llama3v2-1b/tokenizer.json",
            &["worker/llama3v2-1b/model.safetensors"],
        ),
    );
    model.load()?;

    let start_time = Instant::now();
    let result = model.generate("Write me a poem".to_string())?;
    println!("{}\nin {}ms", result, start_time.elapsed().as_millis());

    Ok(())
}
