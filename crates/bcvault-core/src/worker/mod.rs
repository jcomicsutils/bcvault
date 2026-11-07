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
    bandcamp::{BandcampClient, gate::BandcampGate},
    db::DbPool,
    error::DbError,
    paths::AppPaths,
    worker::{
        bridge::{RepaintHook, UiBridge},
        messages::{UiToWorkerCommand, WorkerToUiEvent},
        supervisor::Supervisor,
    },
};
use std::sync::Arc;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};

pub struct WorkerHandle {
    pub cmd_tx: UnboundedSender<UiToWorkerCommand>,
    pub ev_rx: UnboundedReceiver<WorkerToUiEvent>,
    pub db: DbPool,
    pub gate: Arc<BandcampGate>,
    pub client: BandcampClient,
}

pub fn spawn_worker(
    rt: &tokio::runtime::Handle,
    paths: AppPaths,
    repaint_hook: Option<RepaintHook>,
) -> Result<WorkerHandle, DbError> {
    let pool = DbPool::open(&paths.db_path)?;
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    let bridge = UiBridge::new(ev_tx, repaint_hook);

    let bridge_for_gate = bridge.clone();
    let gate = BandcampGate::new(
        1500,
        25,
        2,
        Some(Arc::new(move |st| {
            bridge_for_gate.emit(WorkerToUiEvent::RateLimit(st));
        })),
    );
    let client = BandcampClient::new(gate.clone(), "")
        .map_err(|e| DbError::InvalidEnum(format!("HTTP client init: {e}")))?;

    let pool_for_sup = pool.clone();
    let client_for_sup = client.clone();
    rt.spawn(async move {
        Supervisor::run(paths, pool_for_sup, client_for_sup, bridge, cmd_rx).await;
    });

    Ok(WorkerHandle {
        cmd_tx,
        ev_rx,
        db: pool,
        gate: Arc::new(gate),
        client,
    })
}
