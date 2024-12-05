use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    version = "0.1.0",
    about = "Geirmund master coordinates workers and serves client gateway",
    long_about = None
)]
pub struct Args {
    #[arg(short, long, default_value_t = 8080, help = "Http client gateway port")]
    pub client_port: usize,

    #[arg(short, long, default_value_t = 4339, help = "Tcp workers gateway port")]
    pub workers_port: usize,

    #[arg(short, long, help = "Optional setting for logging into file")]
    pub logfile: Option<String>,

    #[arg(short, long, default_value_t = true, help = "Enable verbose logging")]
    pub verbose: bool,
}
