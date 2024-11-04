use worker::llama::LLama;
use worker::TextModel;
use anyhow::Result;

fn main() -> Result<()> {
    let mut model = LLama::default();
    let result = model.generate("Write recipe for pancakes".to_string())?;
    println!("{}", result);

    Ok(())
}
