use crate::{
    types::{ItemKey, LogEntry, LogLevel},
    worker::messages::WorkerToUiEvent,
};
use std::sync::Arc;
use tokio::sync::mpsc::UnboundedSender;

pub type RepaintHook = Arc<dyn Fn() + Send + Sync>;

#[derive(Clone)]
pub struct UiBridge {
    tx: UnboundedSender<WorkerToUiEvent>,
    repaint_hook: Option<RepaintHook>,
}

impl UiBridge {
    pub fn new(tx: UnboundedSender<WorkerToUiEvent>, repaint_hook: Option<RepaintHook>) -> Self {
        Self { tx, repaint_hook }
    }

    pub fn emit(&self, ev: WorkerToUiEvent) {
        let _ = self.tx.send(ev);
        if let Some(hook) = &self.repaint_hook {
            hook();
        }
    }

    pub fn log(
        &self,
        level: LogLevel,
        scope: &'static str,
        message: impl Into<String>,
        item: Option<ItemKey>,
    ) {
        let ts = chrono::Utc::now().timestamp();
        self.emit(WorkerToUiEvent::Log(LogEntry {
            ts,
            level,
            scope,
            message: message.into(),
            item,
        }));
    }
}
