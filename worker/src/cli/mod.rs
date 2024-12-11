use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    version = "0.1.0", 
    about = "Geirmund worker serves partial inference for LLM models", 
    long_about = None
)]
pub struct Args {
    #[arg(
        short = 'H',
        long,
        default_value = "localhost",
        help = "Master server host"
    )]
    pub host: String,

    #[arg(short, long, default_value = "3000", help = "Master server port")]
    pub port: usize,

    #[arg(
        short,
        long,
        default_value = "llama3v2-1b/config.json",
        help = "Path for json config file"
    )]
    pub config: String,

    #[arg(
        short,
        long,
        default_value = "llama3v2-1b/tokenizer.json",
        help = "Path for json tokenizer file"
    )]
    pub tokenizer: String,

    #[arg(
        short,
        long,
        value_parser,
        num_args = 1..,
        value_delimiter = ' ',
        default_value = "gpt2/gpt2.onnx",
        help = "Paths for weight files (might be multiple)"
    )]
    pub weights: Vec<String>,

    #[arg(
        short = 'd',
        long,
        help = "Optional setting for cuda support (starting from 0)"
    )]
    pub cuda_device: Option<usize>,
}
