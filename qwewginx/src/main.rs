use std::path::PathBuf;

use clap::Parser;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "qwewginx", about = "nginx-ish proxy, pet project edition")]
struct Cli {
    /// config file
    #[arg(short = 'c', long = "config")]
    config: PathBuf,

    /// parse config and print ast, then exit
    #[arg(long = "print-ast", default_value_t = false)]
    print_ast: bool,
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive("qwewginx=info".parse()?))
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let cfg = qwewginx_core::config::parse_file(&cli.config)?;

    if cli.print_ast {
        println!("{cfg:#?}");
        return Ok(());
    }

    info!("loaded {}", cli.config.display());

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;

    rt.block_on(qwewginx_core::server::run(cfg))?;
    Ok(())
}
