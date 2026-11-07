pub mod bridge;
pub mod downloads;
pub mod extract;
pub mod messages;
pub mod quota;
pub mod stream;
pub mod supervisor;
pub mod sync;
pub mod thumbs;

use crate::{
    db::DbPool,
    error::DbError,
    paths::AppPaths,
    worker::{
        bridge::UiBridge,
        messages::{UiToWorkerCommand, WorkerToUiEvent},
        supervisor::Supervisor,
    },
};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

pub struct WorkerHandle {
    pub cmd_tx: UnboundedSender<UiToWorkerCommand>,
    pub ev_rx: UnboundedReceiver<WorkerToUiEvent>,
}

pub fn spawn_worker(
    rt: &tokio::runtime::Handle,
    paths: AppPaths,
    ctx: Option<egui::Context>,
) -> Result<WorkerHandle, DbError> {
    let pool = DbPool::open(&paths.db_path)?;
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    let bridge = UiBridge::new(ev_tx, ctx);

    rt.spawn(async move {
        Supervisor::run(paths, pool, bridge, cmd_rx).await;
    });

    Ok(WorkerHandle { cmd_tx, ev_rx })
}
