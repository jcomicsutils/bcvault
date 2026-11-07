use bcvault::{app::App, paths::AppPaths, worker::spawn_worker};
use std::time::Duration;
use tracing_subscriber::EnvFilter;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let paths = AppPaths::resolve()
        .ok_or_else(|| anyhow::anyhow!("Could not resolve project directories"))?;
    paths.ensure_dirs()?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()?;
    let handle = rt.handle().clone();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1360.0, 820.0])
            .with_min_inner_size([960.0, 560.0]),
        persist_window: true,
        ..Default::default()
    };

    let res = eframe::run_native(
        "bcvault",
        native_options,
        Box::new(move |cc| {
            let worker = spawn_worker(&handle, paths, Some(cc.egui_ctx.clone()))?;
            Ok(Box::new(App::new(cc, handle, worker)))
        }),
    );

    rt.shutdown_timeout(Duration::from_secs(2));

    if let Err(e) = res {
        anyhow::bail!("eframe error: {e}");
    }
    Ok(())
}
