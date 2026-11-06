use crate::ui;

pub struct App {
    _rt_handle: tokio::runtime::Handle,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, rt_handle: tokio::runtime::Handle) -> Self {
        ui::theme::apply(&cc.egui_ctx);
        egui_extras::install_image_loaders(&cc.egui_ctx);
        Self {
            _rt_handle: rt_handle,
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.label("Toolbar");
        });
        egui::TopBottomPanel::bottom("dock")
            .resizable(true)
            .default_height(180.0)
            .show(ctx, |ui| {
                ui.label("Dock");
            });
        egui::SidePanel::left("artists")
            .resizable(true)
            .default_width(280.0)
            .show(ctx, |ui| {
                ui.label("Artists");
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("Releases");
        });
    }
}
