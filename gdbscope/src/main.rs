use anyhow::Result;
use clap::Parser;

use gdbscope::cli::Args;
use gdbscope::config::Config;
use gdbscope::gdb::controller::GdbController;
use gdbscope::state;

fn main() -> Result<()> {
    let args = Args::parse();
    let cfg = Config::from_args(args)?;

    if cfg.debug {
        let filter = tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("gdbscope=debug"));
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_writer(std::io::stderr)
            .init();
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(run(cfg))
}

async fn run(cfg: Config) -> Result<()> {
    let shared = state::new_shared();

    let (cmd_tx, handle) = GdbController::spawn(&cfg, shared.clone()).await?;

    let redraw_hz = cfg.redraw_hz;
    let tui_result = gdbscope::tui::run(shared, cmd_tx.clone(), redraw_hz).await;

    // TUI exited — tell the controller to shut down (detach + exit GDB).
    let _ = cmd_tx
        .send(gdbscope::gdb::controller::GdbCommand::Quit)
        .await;

    // Drop the sender so the controller sees channel-closed if Quit races.
    drop(cmd_tx);

    // Wait for the controller to finish its shutdown sequence.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), handle).await;

    tui_result
}
