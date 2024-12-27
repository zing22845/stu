mod app;
mod cache;
mod client;
mod color;
mod config;
mod constant;
mod environment;
mod error;
mod event;
mod file;
mod macros;
mod object;
mod pages;
mod run;
mod ui;
mod util;
mod widget;

use clap::{arg, Parser, ValueEnum};
use event::AppEventType;
use file::open_or_create_append_file;
use ratatui::{backend::Backend, Terminal};
use std::sync::Mutex;
use tokio::spawn;
use tracing_subscriber::fmt::time::ChronoLocal;

use crate::app::{App, AppContext};
use crate::client::Client;
use crate::color::ColorTheme;
use crate::config::Config;
use crate::environment::Environment;

#[derive(Debug, Clone, Copy, ValueEnum)]
enum PathStyle {
    Auto,
    Always,
    Never,
}

impl From<PathStyle> for client::AddressingStyle {
    fn from(style: PathStyle) -> Self {
        match style {
            PathStyle::Auto => client::AddressingStyle::Auto,
            PathStyle::Always => client::AddressingStyle::Path,
            PathStyle::Never => client::AddressingStyle::VirtualHosted,
        }
    }
}

/// STU - S3 Terminal UI
#[derive(Parser, Debug)]
#[command(version)]
struct Args {
    /// AWS region
    #[arg(short, long)]
    region: Option<String>,

    /// AWS endpoint url
    #[arg(short, long, value_name = "URL")]
    endpoint_url: Option<String>,

    /// AWS profile name
    #[arg(short, long, value_name = "NAME")]
    profile: Option<String>,

    /// Target bucket name
    #[arg(short, long, value_name = "NAME")]
    bucket: Option<String>,

    /// Target prefix
    #[arg(short = 'x', long, value_name = "PREFIX")]
    prefix: Option<String>,

    /// Path style type for object paths
    #[arg(long, value_name = "TYPE", default_value = "auto")]
    path_style: PathStyle,

    /// Enable debug logs
    #[arg(long)]
    debug: bool,
}

fn process_prefix(prefix: Option<String>) -> Option<String> {
    prefix.clone().map(|p| p.trim_end_matches('/').to_string())
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let config = Config::load()?;
    let env = Environment::new(&config);
    let theme = ColorTheme::default();
    let ctx = AppContext::new(config, env, theme);

    initialize_debug_log(&args, &ctx.config)?;

    let mut terminal = ratatui::try_init()?;
    let ret = run(&mut terminal, args, ctx).await;

    ratatui::try_restore()?;

    ret
}

async fn run<B: Backend>(
    terminal: &mut Terminal<B>,
    args: Args,
    ctx: AppContext,
) -> anyhow::Result<()> {
    let (tx, rx) = event::new();
    let (width, height) = get_frame_size(terminal);
    let default_region_fallback = ctx.config.default_region.clone();

    let mut app = App::new(ctx, tx.clone(), width, height);

    spawn(async move {
        let region = args.region.clone();
        let client = Client::new(
            args.region,
            args.endpoint_url,
            args.profile,
            default_region_fallback,
            args.path_style.into(),
        )
        .await;
        let bucket = args.bucket.clone();
        let prefix = process_prefix(args.prefix);
        tx.send(AppEventType::Initialize(client, bucket, prefix, region));
    });

    run::run(&mut app, terminal, rx).await?;

    Ok(())
}

fn get_frame_size<B: Backend>(terminal: &mut Terminal<B>) -> (usize, usize) {
    let size = terminal.get_frame().area();
    (size.width as usize, size.height as usize)
}

fn initialize_debug_log(args: &Args, config: &Config) -> anyhow::Result<()> {
    if args.debug {
        let path = config.debug_log_path()?;
        let file = open_or_create_append_file(path)?;
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_timer(ChronoLocal::rfc_3339())
            .with_file(true)
            .with_line_number(true)
            .with_max_level(tracing::Level::DEBUG)
            .with_writer(Mutex::new(file))
            .init();
    }
    Ok(())
}
