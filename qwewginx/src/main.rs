use std::path::PathBuf;
use std::process::{Child, Command};

use clap::{Parser, ValueEnum};
use qwewginx_core::config::Config;
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Clone, Copy, Debug, Default, ValueEnum)]
enum LogLevel {
    Trace,
    Debug,
    #[default]
    Info,
    Warn,
    Error,
}

impl LogLevel {
    fn directive(self) -> String {
        format!("qwewginx={}", self.filter_str())
    }

    fn filter_str(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        }
    }
}

#[derive(Parser)]
#[command(name = "qwewginx", about = "nginx-ish proxy, pet project edition")]
struct Cli {
    /// config file
    #[arg(short = 'c', long = "config")]
    config: PathBuf,

    /// log level for qwewginx (RUST_LOG still applies to other crates)
    #[arg(short = 'l', long = "log-level", value_enum, default_value_t = LogLevel::Info)]
    log_level: LogLevel,

    /// parse config and print ast, then exit
    #[arg(long = "print-ast", default_value_t = false)]
    print_ast: bool,

    /// internal: run as worker child (set by master)
    #[arg(long, hide = true)]
    worker: bool,
}

fn init_tracing(log_level: LogLevel) -> anyhow::Result<()> {
    let mut filter = EnvFilter::from_default_env();
    filter = filter.add_directive(log_level.directive().parse()?);
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.log_level)?;
    let cfg = qwewginx_core::config::parse_file(&cli.config)?;

    if cli.print_ast {
        println!("{cfg:#?}");
        return Ok(());
    }

    info!("loaded {}", cli.config.display());

    if cli.worker {
        run_worker(cfg)?;
    } else {
        run_master(&cli.config, cli.log_level, cfg)?;
    }

    Ok(())
}

fn run_worker(cfg: Config) -> anyhow::Result<()> {
    info!("worker {} starting", std::process::id());
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(qwewginx_core::server::run(cfg))?;
    Ok(())
}

fn run_master(config_path: &PathBuf, log_level: LogLevel, cfg: Config) -> anyhow::Result<()> {
    let n = cfg.worker_processes.max(1);
    let exe = std::env::current_exe()?;
    info!("master {} spawning {n} workers", std::process::id());

    let mut children: Vec<Child> = Vec::new();
    for _ in 0..n {
        let child = Command::new(&exe)
            .arg("--worker")
            .arg("-c")
            .arg(config_path)
            .arg("--log-level")
            .arg(log_level.filter_str())
            .spawn()?;
        children.push(child);
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(wait_shutdown_signal());

    info!("stopping workers");
    for child in &mut children {
        let _ = child.kill();
    }
    for mut child in children {
        let _ = child.wait();
    }

    info!("master exiting");
    Ok(())
}

async fn wait_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut term =
            signal(SignalKind::terminate()).expect("failed to install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = term.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install ctrl-c handler");
    }
    info!("shutdown signal received");
}
