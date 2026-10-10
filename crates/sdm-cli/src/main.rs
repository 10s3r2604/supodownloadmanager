//! `sdm`: Supo Download Manager on the command line.

mod parse;
mod ui;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Args, Parser, Subcommand};
use indicatif::HumanBytes;
use sdm_core::{
    ClientConfig, Destination, DownloadOptions, DownloadRequest, Downloader, Outcome, ProxyConfig,
};
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::ui::Ui;

#[derive(Parser)]
#[command(
    name = "sdm",
    version,
    about = "Supo Download Manager: fast, resumable, multi-connection downloads",
    after_help = "Press Ctrl+C to pause. Run the same command again to resume."
)]
struct Cli {
    /// Show more log output (-v: warnings, -vv: details)
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Download one or more files
    #[command(visible_alias = "download")]
    Get(GetArgs),
    /// Show what the server says about a file without downloading it
    Info(InfoArgs),
}

#[derive(Args)]
struct GetArgs {
    /// URLs to download
    #[arg(value_name = "URL", required_unless_present = "input_file")]
    urls: Vec<String>,

    /// Read URLs from a file, one per line (lines starting with # are ignored)
    #[arg(short, long, value_name = "FILE")]
    input_file: Option<PathBuf>,

    /// Save to this file (only with a single URL)
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,

    /// Directory to save files into
    #[arg(short, long, value_name = "DIR", default_value = ".")]
    dir: PathBuf,

    /// Connections per file
    #[arg(short, long, value_name = "N", default_value_t = 8,
          value_parser = clap::value_parser!(u16).range(1..=32))]
    connections: u16,

    /// Files to download at the same time
    #[arg(short = 'j', long, value_name = "N", default_value_t = 3,
          value_parser = clap::value_parser!(u16).range(1..=16))]
    parallel: u16,

    /// Limit the total speed, e.g. 500K, 2M or 1.5MB (per second)
    #[arg(short, long, value_name = "SPEED", value_parser = parse::rate)]
    limit_rate: Option<u64>,

    /// How often a connection retries before giving up
    #[arg(long, value_name = "N", default_value_t = 6)]
    retries: u32,

    /// Check the finished file against this SHA-256 checksum
    #[arg(long, value_name = "HEX", value_parser = parse::sha256)]
    sha256: Option<String>,

    /// Replace existing files instead of saving as "name (1).ext"
    #[arg(long)]
    overwrite: bool,

    /// Do not show progress bars
    #[arg(short, long)]
    quiet: bool,

    #[command(flatten)]
    http: HttpArgs,
}

#[derive(Args)]
struct InfoArgs {
    /// URL to look up
    url: String,

    #[command(flatten)]
    http: HttpArgs,
}

#[derive(Args)]
struct HttpArgs {
    /// Extra request header, e.g. -H "Authorization: Bearer TOKEN" (repeatable)
    #[arg(short = 'H', long = "header", value_name = "NAME: VALUE", value_parser = parse::header)]
    headers: Vec<(String, String)>,

    /// User-Agent to send
    #[arg(short = 'U', long, value_name = "AGENT")]
    user_agent: Option<String>,

    /// Referer to send
    #[arg(long, value_name = "URL")]
    referer: Option<String>,

    /// Cookies to send, e.g. "session=abc; lang=vi"
    #[arg(long, value_name = "COOKIES")]
    cookie: Option<String>,

    /// Proxy URL (http://, https:// or socks5://), or "none" to ignore the system proxy
    #[arg(long, value_name = "URL")]
    proxy: Option<String>,
}

impl HttpArgs {
    fn client_config(&self, speed_limit: Option<u64>) -> ClientConfig {
        let mut config = ClientConfig {
            speed_limit,
            ..ClientConfig::default()
        };
        if let Some(agent) = &self.user_agent {
            config.user_agent = agent.clone();
        }
        config.proxy = match self.proxy.as_deref() {
            None => ProxyConfig::System,
            Some("none") => ProxyConfig::Disabled,
            Some(url) => ProxyConfig::Url(url.to_owned()),
        };
        config
    }

    fn headers(&self) -> Vec<(String, String)> {
        let mut headers = self.headers.clone();
        if let Some(referer) = &self.referer {
            headers.push(("Referer".into(), referer.clone()));
        }
        if let Some(cookie) = &self.cookie {
            headers.push(("Cookie".into(), cookie.clone()));
        }
        headers
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    init_logging(cli.verbose);
    let result = match cli.command {
        Command::Get(args) => get(args).await,
        Command::Info(args) => info(args).await,
    };
    result.unwrap_or_else(|error| {
        eprintln!("error: {error:#}");
        ExitCode::FAILURE
    })
}

fn init_logging(verbose: u8) {
    let level = match verbose {
        0 => "error",
        1 => "warn",
        _ => "debug",
    };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(format!("sdm_core={level}")));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .init();
}

async fn info(args: InfoArgs) -> Result<ExitCode> {
    let downloader = Downloader::new(args.http.client_config(None))?;
    let info = downloader.probe(&args.url, &args.http.headers()).await?;

    println!("File name:  {}", info.file_name);
    match info.size {
        Some(size) => println!("Size:       {} ({size} bytes)", HumanBytes(size)),
        None => println!("Size:       unknown"),
    }
    println!(
        "Type:       {}",
        info.content_type.as_deref().unwrap_or("unknown")
    );
    if info.resumable {
        println!("Resumable:  yes (multiple connections, can pause and resume)");
    } else {
        println!("Resumable:  no (single connection, starts over if interrupted)");
    }
    if let Some(modified) = &info.last_modified {
        println!("Modified:   {modified}");
    }
    if info.final_url != info.url {
        println!("Redirected: {}", info.final_url);
    }
    Ok(ExitCode::SUCCESS)
}

async fn get(args: GetArgs) -> Result<ExitCode> {
    let mut urls = args.urls.clone();
    if let Some(file) = &args.input_file {
        urls.extend(read_url_list(file)?);
    }
    if urls.is_empty() {
        bail!("no URLs to download");
    }
    if urls.len() > 1 && args.output.is_some() {
        bail!("--output works with a single URL; use --dir for several");
    }
    if urls.len() > 1 && args.sha256.is_some() {
        bail!("--sha256 works with a single URL");
    }

    let downloader = Downloader::new(args.http.client_config(args.limit_rate))?;
    let headers = args.http.headers();
    let options = DownloadOptions {
        connections: args.connections.into(),
        max_retries: args.retries,
        overwrite: args.overwrite,
        ..DownloadOptions::default()
    };
    let ui = Ui::new(args.quiet);
    let interrupted = CancellationToken::new();
    tokio::spawn(watch_ctrl_c(interrupted.clone(), ui.clone()));

    let slots = Arc::new(Semaphore::new(args.parallel.into()));
    let mut tasks = JoinSet::new();
    for url in urls {
        let destination = match &args.output {
            Some(path) => Destination::File(path.clone()),
            None => Destination::Directory(args.dir.clone()),
        };
        let request = DownloadRequest {
            url,
            destination,
            headers: headers.clone(),
            options: options.clone(),
        };
        tasks.spawn(download(Job {
            downloader: downloader.clone(),
            request,
            slots: slots.clone(),
            ui: ui.clone(),
            interrupted: interrupted.clone(),
            sha256: args.sha256.clone(),
        }));
    }

    let mut failed = 0;
    while let Some(result) = tasks.join_next().await {
        if result.unwrap_or(Finish::Failed) == Finish::Failed {
            failed += 1;
        }
    }

    Ok(if interrupted.is_cancelled() {
        ExitCode::from(130)
    } else if failed > 0 {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Finish {
    Done,
    Paused,
    Failed,
}

struct Job {
    downloader: Downloader,
    request: DownloadRequest,
    slots: Arc<Semaphore>,
    ui: Ui,
    interrupted: CancellationToken,
    sha256: Option<String>,
}

async fn download(job: Job) -> Finish {
    let Job {
        downloader,
        request,
        slots,
        ui,
        interrupted,
        sha256,
    } = job;
    let url = request.url.clone();
    let bar = ui.add_bar(&ui::label_for_url(&url));

    let _slot = tokio::select! {
        slot = slots.acquire_owned() => slot.expect("semaphore is never closed"),
        _ = interrupted.cancelled() => {
            bar.finish_and_clear();
            return Finish::Paused;
        }
    };

    let started = Instant::now();
    let handle = downloader.start(request);
    let control = handle.control();
    let mut finished = std::pin::pin!(handle.wait());
    let mut ticker = tokio::time::interval(Duration::from_millis(200));
    let mut pausing = false;
    let result = loop {
        tokio::select! {
            result = &mut finished => break result,
            _ = ticker.tick() => ui::update_bar(&bar, &control.progress()),
            _ = interrupted.cancelled(), if !pausing => {
                control.pause();
                pausing = true;
            }
        }
    };
    bar.finish_and_clear();

    let progress = control.progress();
    let name = progress
        .file_name
        .clone()
        .unwrap_or_else(|| ui::label_for_url(&url));
    match result {
        Ok(Outcome::Completed { path, size }) => {
            if let Some(expected) = sha256 {
                match sdm_core::sha256_file(&path).await {
                    Ok(actual) if actual == expected => {}
                    Ok(actual) => {
                        ui.line(format!(
                            "✗ {}: checksum mismatch (expected {expected}, got {actual})",
                            path.display()
                        ));
                        return Finish::Failed;
                    }
                    Err(error) => {
                        ui.line(format!(
                            "✗ {}: could not verify checksum: {error}",
                            path.display()
                        ));
                        return Finish::Failed;
                    }
                }
            }
            ui.line(format!(
                "✓ {}  {}",
                path.display(),
                ui::summary(size, progress.resumed_from, started.elapsed())
            ));
            Finish::Done
        }
        Ok(Outcome::Paused {
            resumable: true, ..
        }) => {
            let percent = progress
                .fraction()
                .map(|f| format!(" at {:.0}%", f * 100.0))
                .unwrap_or_default();
            ui.line(format!(
                "⏸ {name} paused{percent}. Run the same command again to resume."
            ));
            Finish::Paused
        }
        Ok(Outcome::Paused {
            resumable: false, ..
        })
        | Ok(Outcome::Cancelled) => {
            ui.line(format!(
                "⏸ {name} stopped. The server does not support resuming, so it will start over."
            ));
            Finish::Paused
        }
        Err(error) => {
            ui.line(format!("✗ {name}: {error}"));
            Finish::Failed
        }
    }
}

async fn watch_ctrl_c(interrupted: CancellationToken, ui: Ui) {
    if tokio::signal::ctrl_c().await.is_err() {
        return;
    }
    ui.line("Pausing downloads... (press Ctrl+C again to quit immediately)");
    interrupted.cancel();
    if tokio::signal::ctrl_c().await.is_ok() {
        std::process::exit(130);
    }
}

fn read_url_list(path: &Path) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect())
}
