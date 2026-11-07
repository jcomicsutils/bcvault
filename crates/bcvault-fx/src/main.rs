use bcvault_core::{paths::AppPaths, worker::spawn_worker};
use bcvault_fx::{app::FxApp, services::spawn_fx_services};
use std::{sync::Arc, time::Duration};
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
    let _instance_lock = paths.acquire_instance_lock()?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()?;
    let handle = rt.handle().clone();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("bcvault-fx")
            .with_app_id("bcvault-fx")
            .with_inner_size([1440.0, 860.0])
            .with_min_inner_size([1024.0, 600.0]),
        persist_window: true,
        ..Default::default()
    };

    let res = eframe::run_native(
        "bcvault-fx",
        native_options,
        Box::new(move |cc| {
            let ctx = cc.egui_ctx.clone();
            let repaint_hook: bcvault_core::worker::bridge::RepaintHook =
                Arc::new(move || ctx.request_repaint());
            let core_worker = spawn_worker(&handle, paths.clone(), Some(repaint_hook.clone()))?;
            let fx_worker = spawn_fx_services(
                &handle,
                paths,
                core_worker.db.clone(),
                core_worker.client.clone(),
                Some(repaint_hook),
            )?;
            Ok(Box::new(FxApp::new(
                cc,
                core_worker,
                fx_worker,
                handle.clone(),
            )))
        }),
    );

    rt.shutdown_timeout(Duration::from_secs(2));

    if let Err(e) = res {
        anyhow::bail!("eframe error: {e}");
    }
    Ok(())
}
