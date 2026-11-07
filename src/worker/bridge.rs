use crate::{
    types::{ItemKey, LogEntry, LogLevel},
    worker::messages::WorkerToUiEvent,
};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone)]
pub struct UiBridge {
    tx: UnboundedSender<WorkerToUiEvent>,
    ctx: Option<egui::Context>,
}

impl UiBridge {
    pub fn new(tx: UnboundedSender<WorkerToUiEvent>, ctx: Option<egui::Context>) -> Self {
        Self { tx, ctx }
    }

    pub fn emit(&self, ev: WorkerToUiEvent) {
        let _ = self.tx.send(ev);
        if let Some(ctx) = &self.ctx {
            ctx.request_repaint();
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
