//! Headless benchmark for `bcvault-fx`:
//! Seeds a 100k-row SQLite DB, loads the wall, auto-scrolls for 600 frames,
//! writes frame times to CSV (or stdout), and with `--check` verifies p95 frame time < 6 ms.

use bcvault_core::db::{DbPool, seed};
use bcvault_fx::{
    actions::FxQuery,
    db::{FxDb, query_wall},
    state::FxState,
    theme::apply_theme,
    views,
};
use std::time::Instant;
use tokio::sync::mpsc;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let check_mode = std::env::args().any(|a| a == "--check");
    let dir = tempfile::tempdir()?;
    let main_db = dir.path().join("bcvault.db");
    let fx_db_path = dir.path().join("fx.sqlite");

    let main_pool = DbPool::open(&main_db)?;
    main_pool.writer.call(|c| seed::seed(c, 100_000)).await?;

    let fx_db = FxDb::open(&fx_db_path, &main_db)?;
    let rows = fx_db
        .reader
        .call(|c| {
            let mut q = FxQuery::default();
            q.base.band_id = Some(1);
            query_wall(c, &q)
        })
        .await?;

    let ctx = egui::Context::default();
    let mut state = FxState::new();
    apply_theme(&ctx, &state.active_theme);
    state.rows = rows;

    let (core_tx, _core_rx) = mpsc::unbounded_channel();
    let (fx_tx, _fx_rx) = mpsc::unbounded_channel();

    let mut frame_times_ms = Vec::with_capacity(600);
    for frame in 0..600 {
        state.pending_scroll_to_y = Some((frame as f32) * 18.0);
        let raw_input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::Vec2::new(1440.0, 860.0),
            )),
            predicted_dt: 1.0 / 60.0,
            ..Default::default()
        };
        let t0 = Instant::now();
        let _ = ctx.run(raw_input, |ctx| {
            state.motion.begin_frame_dt(1.0 / 60.0);
            egui::CentralPanel::default().show(ctx, |ui| {
                views::wall::show(ui, &mut state, &core_tx, &fx_tx, (frame as f64) / 60.0);
            });
            state.motion.end_frame(ctx);
        });
        frame_times_ms.push(t0.elapsed().as_secs_f64() * 1000.0);
    }

    frame_times_ms.sort_by(|a, b| a.total_cmp(b));
    let p50 = frame_times_ms[frame_times_ms.len() / 2];
    let p95 = frame_times_ms[(frame_times_ms.len() * 95) / 100];
    println!("fx_bench: 600 frames | p50 = {p50:.3} ms | p95 = {p95:.3} ms");

    if check_mode && p95 > 6.0 {
        anyhow::bail!("p95 frame time {p95:.3} ms exceeded 6.0 ms budget");
    }
    Ok(())
}
