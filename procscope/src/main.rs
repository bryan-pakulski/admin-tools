use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

use procscope::aggregator::{self, AggregatorConfig};
use procscope::capabilities::Capabilities;
use procscope::cli::Args;
use procscope::config::Config;
use procscope::export::CsvLog;
use procscope::sampler;
use procscope::tui;

fn main() -> Result<()> {
    let args = Args::parse();
    let cfg = Config::from_args(args)?;

    if cfg.debug {
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("procscope=debug"));
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .init();
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async move { run(cfg).await })
}

async fn run(cfg: Config) -> Result<()> {
    let caps = Capabilities::probe(cfg.pid);
    tracing::debug!(?caps, "capabilities probed");

    let sampler_handles = sampler::spawn(cfg.pid, cfg.interval);

    let csv_log = match cfg.write_csv.as_deref() {
        Some(path) => Some(CsvLog::create(&PathBuf::from(path))?),
        None => None,
    };

    let agg_cfg = AggregatorConfig {
        pid: cfg.pid,
        interval: cfg.interval,
        caps,
        thresholds: cfg.freeze,
        filter: cfg.filter.clone(),
        recent_cap: cfg.recent_cap,
        max_history: cfg.max_history,
    };

    let agg = aggregator::spawn(
        agg_cfg,
        sampler_handles.samples_rx,
        sampler_handles.paused_tx.subscribe(),
        sampler_handles.interval_tx.subscribe(),
        csv_log,
    );

    let snapshot = agg.snapshot.clone();
    let snapshot_for_exit = snapshot.clone();

    let result = if cfg.no_tui {
        tui::run_no_tui(snapshot, cfg.duration).await
    } else {
        tui::run(
            snapshot,
            cfg.clone(),
            sampler_handles.paused_tx.clone(),
            sampler_handles.interval_tx.clone(),
        )
        .await
    };

    if cfg.export_on_exit {
        let snap = snapshot_for_exit.load_full();
        match procscope::export::write_snapshot(&snap, &PathBuf::from(".")) {
            Ok(path) => eprintln!("wrote snapshot CSV: {}", path.display()),
            Err(e) => eprintln!("export failed: {e}"),
        }
    }

    result
}
