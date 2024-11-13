use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    #[arg(short, long, default_value = "localhost")]
    pub master_host: String,

    #[arg(short, long, default_value = "3000")]
    pub master_port: usize,

    #[arg(short, long, default_value = "llama3v2-1b/config.json")]
    pub config_file: String,

    #[arg(short, long, default_value = "llama3v2-1b/tokenizer.json")]
    pub tokenizer_file: String,

    #[arg(
        short,
        long,
        value_parser,
        num_args = 1..,
        value_delimiter = ' ',
        default_value = "llama3v2-1b/model.safetensors"
    )]
    pub weight_files: Vec<String>,

    #[arg(short, long)]
    pub cuda_device: Option<usize>,
}
