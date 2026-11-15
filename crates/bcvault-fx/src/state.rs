use crate::{
    actions::{FxAction, FxQuery, RemainingTarget, ViewMode},
    db::{Facets, Palette, Prefs, ReleaseDetail, Stats, WallRow},
    motion::{Motion, SpringCfg},
    nav::{NavHistory, SavedView, ViewState},
    services::{
        messages::{FxCommand, FxEvent, SearchHit, SearchScope, Tier, Toast, ToastKind},
        thumbs::TextureCache,
    },
    theme::{FxTheme, registry::ThemeRegistry},
};
use bcvault_core::{
    library::service::ReconcileReport,
    types::{
        ArtistFolder, ArtistView, DeletedPolicy, DownloadOpts, FolderState, ItemKey, JobId,
        JobState, JobView, LogEntry, MissingItem, RateLimitStatus, Settings, SyncMode,
        SyncProgress, SyncScope, UnavailableFolder, UnavailableRoot,
    },
    worker::messages::{UiToWorkerCommand, WorkerToUiEvent},
};
use egui::{Rect, Vec2};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    time::{Duration, Instant},
};
use tokio::sync::mpsc::UnboundedSender;

pub const CARD_SIZES: [f32; 4] = [120.0, 168.0, 232.0, 320.0];
pub const CARD_SIZE_LABELS: [&str; 4] = ["S", "M", "L", "XL"];

#[derive(Clone, Debug)]
pub struct FxJob {
    pub view: JobView,
    pub bytes_done: u64,
    pub bytes_total: Option<u64>,
    pub speed_bps: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FxMissingItemChoice {
    Moved(std::path::PathBuf),
    Deleted,
    Skip,
}

#[derive(Clone)]
pub struct FxResolveMissingModalState {
    pub batch_id: u64,
    pub items: Vec<MissingItem>,
    pub mass_missing: bool,
    pub decisions: HashMap<ItemKey, FxMissingItemChoice>,
    pub deleted_policy: DeletedPolicy,
    pub make_default: bool,
    pub stop_asking: bool,
    pub picked_folder: std::sync::Arc<std::sync::Mutex<Option<(ItemKey, std::path::PathBuf)>>>,
    pub repoint_picked_folder: std::sync::Arc<std::sync::Mutex<Option<(i64, std::path::PathBuf)>>>,
    pub repoint_selected_band_id: Option<i64>,
}

impl FxResolveMissingModalState {
    pub fn new(
        batch_id: u64,
        items: Vec<MissingItem>,
        mass_missing: bool,
        default_policy: DeletedPolicy,
    ) -> Self {
        let mut decisions = HashMap::new();
        for item in &items {
            if let Some(cand) = &item.best_candidate {
                decisions.insert(item.key, FxMissingItemChoice::Moved(cand.path.clone()));
            } else {
                decisions.insert(item.key, FxMissingItemChoice::Deleted);
            }
        }
        let repoint_selected_band_id = items.first().map(|it| it.band_id);
        Self {
            batch_id,
            items,
            mass_missing,
            decisions,
            deleted_policy: default_policy,
            make_default: false,
            stop_asking: false,
            picked_folder: std::sync::Arc::new(std::sync::Mutex::new(None)),
            repoint_picked_folder: std::sync::Arc::new(std::sync::Mutex::new(None)),
            repoint_selected_band_id,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SidebarRowKey {
    AllArtists,
    GroupHeader(i64),
    UngroupedHeader,
    Artist(i64),
}

impl SidebarRowKey {
    pub fn from_row(row: &bcvault_core::groups::SidebarRow) -> Self {
        match row {
            bcvault_core::groups::SidebarRow::AllArtists => Self::AllArtists,
            bcvault_core::groups::SidebarRow::GroupHeader { id, .. } => Self::GroupHeader(*id),
            bcvault_core::groups::SidebarRow::UngroupedHeader { .. } => Self::UngroupedHeader,
            bcvault_core::groups::SidebarRow::Artist { band_id } => Self::Artist(*band_id),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SidebarGhost {
    pub key: SidebarRowKey,
    pub label: String,
    pub is_header: bool,
    pub band_id: Option<i64>,
    pub y_rel: f32,
    pub created_s: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FxUnavailableCardState {
    pub folder: UnavailableFolder,
    pub policy: DeletedPolicy,
    pub unlink: bool,
    pub expanded: bool,
}

#[derive(Clone)]
pub struct FxUnavailableModalState {
    pub batch_id: u64,
    pub cards: Vec<FxUnavailableCardState>,
    pub picked_folder: std::sync::Arc<std::sync::Mutex<Option<(i64, std::path::PathBuf)>>>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FxBulkAction {
    View(Vec<ItemKey>),
    Remaining(bcvault_core::types::RemainingScope),
}

#[derive(Clone, Debug)]
pub struct FxBulkConfirmRow {
    pub artist_name: String,
    pub preview: bcvault_core::worker::quota::BulkPreviewRow,
}

#[derive(Clone, Debug)]
pub struct FxBulkConfirmModalState {
    pub title: String,
    pub rows: Vec<FxBulkConfirmRow>,
    pub total_new: u32,
    pub total_queue: u32,
    pub total_deferred: u32,
    pub action: FxBulkAction,
    pub focus_init: bool,
}

impl FxUnavailableModalState {
    pub fn new(
        batch_id: u64,
        folders: Vec<UnavailableFolder>,
        default_policy: DeletedPolicy,
    ) -> Self {
        let cards = folders
            .into_iter()
            .map(|folder| {
                let unlink = matches!(folder.root, UnavailableRoot::Linked { .. });
                FxUnavailableCardState {
                    folder,
                    policy: default_policy,
                    unlink,
                    expanded: false,
                }
            })
            .collect();
        Self {
            batch_id,
            cards,
            picked_folder: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FxReportTab {
    Matched,
    Review,
    PageOnly,
    LocalOnly,
    Conflicts,
    Restorable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FxLocalAction {
    Leave,
    Ignore,
    Attach(ItemKey),
}

#[derive(Clone)]
pub struct FxReconcileReportModalState {
    pub report: Box<ReconcileReport>,
    pub active_tab: FxReportTab,
    pub selected_matched: std::collections::HashSet<ItemKey>,
    pub selected_review: HashMap<ItemKey, usize>,
    pub rejected_review: std::collections::HashSet<ItemKey>,
    pub selected_restorable: std::collections::HashSet<ItemKey>,
    pub local_decisions: HashMap<std::path::PathBuf, FxLocalAction>,
    pub conflict_decisions: HashMap<std::path::PathBuf, FxLocalAction>,
    pub local_attach_search: HashMap<std::path::PathBuf, String>,
}

impl FxReconcileReportModalState {
    pub fn new(report: Box<ReconcileReport>) -> Self {
        let selected_matched = report.matched.iter().map(|m| m.release_key).collect();
        let selected_restorable = report.restorable.iter().map(|m| m.release_key).collect();
        let mut selected_review = HashMap::new();
        for r in &report.review {
            if !r.candidates.is_empty() {
                selected_review.insert(r.release_key, 0);
            }
        }
        let rejected_review = std::collections::HashSet::new();
        let local_decisions = HashMap::new();
        let conflict_decisions = HashMap::new();
        let local_attach_search = HashMap::new();

        let active_tab = if !report.matched.is_empty() {
            FxReportTab::Matched
        } else if !report.review.is_empty() {
            FxReportTab::Review
        } else if !report.restorable.is_empty() {
            FxReportTab::Restorable
        } else if !report.local_only.is_empty() {
            FxReportTab::LocalOnly
        } else if !report.conflicts.is_empty() {
            FxReportTab::Conflicts
        } else {
            FxReportTab::PageOnly
        };

        Self {
            report,
            active_tab,
            selected_matched,
            selected_review,
            rejected_review,
            selected_restorable,
            local_decisions,
            conflict_decisions,
            local_attach_search,
        }
    }

    pub fn mark_all_review_wrong(&mut self) {
        self.selected_review.clear();
        for r in &self.report.review {
            self.rejected_review.insert(r.release_key);
        }
    }

    pub fn mark_unselected_review_wrong(&mut self) {
        for r in &self.report.review {
            if !self.selected_review.contains_key(&r.release_key) {
                self.rejected_review.insert(r.release_key);
            }
        }
    }

    pub fn skip_all_review(&mut self) {
        self.selected_review.clear();
        self.rejected_review.clear();
    }

    pub fn total_selected_count(&self) -> usize {
        let matched_cnt = self.selected_matched.len();
        let review_cnt = self.selected_review.len() + self.rejected_review.len();
        let restorable_cnt = self.selected_restorable.len();
        let local_cnt = self
            .local_decisions
            .values()
            .filter(|a| !matches!(a, FxLocalAction::Leave))
            .count();
        let conflict_cnt = self
            .conflict_decisions
            .values()
            .filter(|a| !matches!(a, FxLocalAction::Leave))
            .count();
        matched_cnt + review_cnt + restorable_cnt + local_cnt + conflict_cnt
    }

    pub fn build_decisions(&self) -> Vec<bcvault_core::types::MatchDecision> {
        use bcvault_core::types::MatchDecision;
        let mut decisions = Vec::new();

        for m in &self.report.matched {
            if self.selected_matched.contains(&m.release_key) {
                decisions.push(MatchDecision::Adopt {
                    key: m.release_key,
                    path: m.candidate_path.clone(),
                    format: m.format_guess,
                    folder_mtime: None,
                });
            }
        }

        for r in &self.report.review {
            if self.rejected_review.contains(&r.release_key) {
                for cand in &r.candidates {
                    decisions.push(MatchDecision::Reject {
                        band_id: self.report.band_id,
                        key: r.release_key,
                        path: cand.candidate_path.clone(),
                    });
                }
            } else if let Some(cand) = self
                .selected_review
                .get(&r.release_key)
                .and_then(|&cand_idx| r.candidates.get(cand_idx))
            {
                decisions.push(MatchDecision::Adopt {
                    key: r.release_key,
                    path: cand.candidate_path.clone(),
                    format: cand.format_guess,
                    folder_mtime: None,
                });
            }
        }

        for m in &self.report.restorable {
            if self.selected_restorable.contains(&m.release_key) {
                decisions.push(MatchDecision::Restore {
                    key: m.release_key,
                    path: m.candidate_path.clone(),
                });
            }
        }

        for (path, action) in &self.local_decisions {
            match action {
                FxLocalAction::Ignore => {
                    decisions.push(MatchDecision::Ignore {
                        band_id: self.report.band_id,
                        path: path.clone(),
                    });
                }
                FxLocalAction::Attach(key) => {
                    let format_guess = self
                        .report
                        .local_only
                        .iter()
                        .find(|o| &o.path == path)
                        .and_then(|o| o.format_guess);
                    decisions.push(MatchDecision::Attach {
                        key: *key,
                        path: path.clone(),
                        format: format_guess,
                        folder_mtime: None,
                    });
                }
                FxLocalAction::Leave => {}
            }
        }

        for (path, action) in &self.conflict_decisions {
            match action {
                FxLocalAction::Ignore => {
                    decisions.push(MatchDecision::Ignore {
                        band_id: self.report.band_id,
                        path: path.clone(),
                    });
                }
                FxLocalAction::Attach(key) => {
                    decisions.push(MatchDecision::Attach {
                        key: *key,
                        path: path.clone(),
                        format: None,
                        folder_mtime: None,
                    });
                }
                FxLocalAction::Leave => {}
            }
        }

        decisions
    }
}

#[derive(Clone, Debug)]
pub struct FxLinkFolderModalState {
    pub band_id: Option<i64>,
    pub artist_input: String,
    pub folder_path: String,
    pub auto_apply_confident: bool,
    pub picked_folder: std::sync::Arc<std::sync::Mutex<Option<std::path::PathBuf>>>,
    pub error: Option<String>,
    pub repoint_folder_id: Option<i64>,
}

impl FxLinkFolderModalState {
    pub fn new(band_id: Option<i64>, folder_path: String) -> Self {
        Self {
            band_id,
            artist_input: String::new(),
            folder_path,
            auto_apply_confident: true,
            picked_folder: std::sync::Arc::new(std::sync::Mutex::new(None)),
            error: None,
            repoint_folder_id: None,
        }
    }

    pub fn new_empty(band_id: Option<i64>) -> Self {
        Self::new(band_id, String::new())
    }

    pub fn for_repoint(band_id: i64, folder_id: i64, folder_path: String) -> Self {
        Self {
            band_id: Some(band_id),
            artist_input: String::new(),
            folder_path,
            auto_apply_confident: true,
            picked_folder: std::sync::Arc::new(std::sync::Mutex::new(None)),
            error: None,
            repoint_folder_id: Some(folder_id),
        }
    }
}

pub fn worst_folder_state(folders: &[ArtistFolder]) -> Option<bcvault_core::types::FolderState> {
    if folders.is_empty() {
        return None;
    }
    if folders
        .iter()
        .any(|f| f.state == bcvault_core::types::FolderState::Missing)
    {
        Some(bcvault_core::types::FolderState::Missing)
    } else if folders
        .iter()
        .any(|f| f.state == bcvault_core::types::FolderState::Unreadable)
    {
        Some(bcvault_core::types::FolderState::Unreadable)
    } else {
        Some(bcvault_core::types::FolderState::Ok)
    }
}

pub fn folder_display_label(folder: &ArtistFolder, all_folders: &[ArtistFolder]) -> String {
    let base_name = folder
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| folder.path.display().to_string());
    let duplicate_count = all_folders
        .iter()
        .filter(|f| f.path.file_name().map(|n| n.to_string_lossy()).as_deref() == Some(&base_name))
        .count();
    if duplicate_count > 1 {
        if let Some(parent) = folder.path.parent()
            && let Some(p_name) = parent.file_name()
        {
            format!("{}/{}", p_name.to_string_lossy(), base_name)
        } else {
            base_name
        }
    } else {
        base_name
    }
}

#[derive(Clone, Debug)]
pub struct FxQuotaWarningEntry {
    pub band_id: i64,
    pub used_30d: u32,
    pub requested: u32,
    pub soft: u32,
    pub hard: u32,
    pub items: Vec<ItemKey>,
    pub opts: DownloadOpts,
}

pub struct FxState {
    pub booted: bool,
    pub schema_error: Option<String>,
    pub artists: Vec<ArtistView>,
    pub settings: Settings,
    pub prefs: Prefs,
    pub themes: ThemeRegistry,
    pub themes_dir: Option<std::path::PathBuf>,
    pub theme_preview: Option<FxTheme>,
    pub theme_editor: Option<crate::overlays::theme_editor::ThemeEditorState>,
    pub pending_theme_import_pick: bool,
    pub theme_picked_import: std::sync::Arc<std::sync::Mutex<Option<Vec<std::path::PathBuf>>>>,
    pub pending_theme_export_pick: Option<crate::theme::file::ThemeFile>,
    pub theme_picked_export: std::sync::Arc<
        std::sync::Mutex<Option<(crate::theme::file::ThemeFile, std::path::PathBuf)>>,
    >,
    pub active_theme: FxTheme,
    pub motion: Motion,
    pub tex_cache: TextureCache,
    pub palettes: HashMap<i64, Palette>,
    pub cover_cache_bytes: u64,

    // Navigation & View
    pub view_mode: ViewMode,
    pub prev_view_mode: ViewMode,
    pub view_switch_seq: u64,
    pub query: FxQuery,
    pub nav: NavHistory,
    pub saved_views: Vec<SavedView>,
    pub recents: Vec<(String, String)>,
    pub sidebar_collapsed: bool,
    pub scroll_y: f32,
    pub pending_scroll_to_y: Option<f32>,

    // Query & Streaming rows
    pub next_request_id: u64,
    pub active_wall_req_id: u64,
    pub active_facets_req_id: u64,
    pub active_stats_req_id: u64,
    pub active_search_req_id: u64,
    pub query_dirty_at: Option<Instant>,
    pub rows: Vec<WallRow>,
    pub facets: Facets,
    pub stats: Stats,
    pub thumb_generation: u64,
    pub last_thumb_req_at: Instant,

    // Selection, Focus, Hover & FLIP
    pub selected_keys: BTreeSet<ItemKey>,
    pub last_clicked_key: Option<ItemKey>,
    pub focused_key: Option<ItemKey>,
    pub hovered_key: Option<ItemKey>,
    pub hovered_art_id: Option<i64>,
    pub marquee_start: Option<egui::Pos2>,
    pub marquee_current: Option<egui::Pos2>,
    pub last_card_rects: HashMap<ItemKey, Rect>,
    pub flip_origins: HashMap<ItemKey, Rect>,
    pub flip_seed_pending: bool,
    pub flip_ghosts: Vec<(ItemKey, Option<i64>, Rect, f64)>,
    pub row_flashes: HashMap<ItemKey, f64>,
    pub artist_dive_banner: Option<(i64, Rect)>,

    // Detail Drawer
    pub drawer_key: Option<ItemKey>,
    pub drawer_opened_at: Option<Instant>,
    pub drawer_detail_requested: bool,
    pub details_cache: HashMap<ItemKey, ReleaseDetail>,
    pub drawer_slide_dir: f32,

    // Command Palette
    pub palette_open: bool,
    pub palette_query: String,
    pub palette_scope: SearchScope,
    pub palette_hits: Vec<SearchHit>,
    pub palette_sel: usize,
    pub focus_search_next_frame: bool,
    pub focus_palette_next_frame: bool,

    // Sidebar / Sync / Modals
    pub add_artist_input: String,
    pub add_artist_pending: bool,
    pub artist_filter: String,
    pub sync_mode: SyncMode,
    pub active_syncs: HashMap<i64, SyncProgress>,
    pub redeem_modal: Option<(ItemKey, String, String)>,
    pub save_view_modal: Option<String>,
    pub settings_modal_open: bool,
    pub settings_draft: Option<Settings>,
    pub settings_confirm_discard: bool,
    pub settings_focus_first: bool,
    pub settings_focus_keep_editing: bool,
    pub settings_tab_anim_dir: f32,
    pub settings_tab_anim_gen: u64,
    pub settings_folders_filter: String,
    pub settings_picked_out_dir: std::sync::Arc<std::sync::Mutex<Option<std::path::PathBuf>>>,
    pub filename_pattern_input: String,
    pub confirm_remove_artist: Option<(i64, String)>,
    pub rename_artist_modal: Option<(i64, String, String)>,
    pub link_folder_modal: Option<FxLinkFolderModalState>,
    pub reconcile_report_modal: Option<FxReconcileReportModalState>,
    pub resolve_missing_modal: Option<FxResolveMissingModalState>,
    pub show_resolve_missing_window: bool,
    pub unavailable_modal: Option<FxUnavailableModalState>,
    pub show_unavailable_window: bool,
    pub folders: HashMap<i64, Vec<ArtistFolder>>,
    pub relink_picked_folder:
        std::sync::Arc<std::sync::Mutex<Option<(ItemKey, std::path::PathBuf)>>>,
    pub pending_relink_pick: Option<ItemKey>,

    // Artist Groups & Multi-Select
    pub active_grouping: bcvault_core::groups::ActiveGrouping,
    pub group_sets: Vec<bcvault_core::groups::ArtistGroupSet>,
    pub groups: Vec<bcvault_core::groups::ArtistGroup>,
    pub builtin_groups: Vec<bcvault_core::groups::ArtistGroup>,
    pub collapsed_builtin_groups: BTreeSet<i64>,
    pub pins: HashMap<i64, i64>,
    pub selected_artists: BTreeSet<i64>,
    pub last_clicked_artist: Option<i64>,
    pub group_assignment: bcvault_core::groups::Assignment,
    pub sidebar_rows: Vec<bcvault_core::groups::SidebarRow>,
    pub sidebar_flip_active: HashSet<SidebarRowKey>,
    pub sidebar_enter_active: HashSet<SidebarRowKey>,
    pub sidebar_ghosts: Vec<SidebarGhost>,
    pub sidebar_anim_initialized: bool,
    pub ungrouped_collapsed: bool,
    pub sidebar_dirty: bool,
    pub sidebar_last_filter: String,
    pub sidebar_last_recompute_ts: i64,
    pub sidebar_recompute_count: u64,
    pub group_error: Option<String>,
    pub confirm_delete_group: Option<(i64, String, usize)>,
    pub confirm_delete_group_set: Option<(i64, String)>,
    pub group_editor: Option<crate::overlays::groups::FxGroupEditorState>,
    pub group_set_modal: Option<crate::overlays::groups::FxGroupSetModalState>,
    pub pending_group_pins: Option<(String, Vec<i64>)>,

    // Jobs, Speed Sparkline, Rate limit, Toasts & Logs
    pub jobs: Vec<FxJob>,
    pub speed_samples: std::collections::VecDeque<f64>,
    pub last_speed_sample_at: Instant,
    pub completed_jobs_at: HashMap<JobId, f64>,
    pub rate_limit: RateLimitStatus,
    pub toasts: Vec<Toast>,
    pub next_toast_id: u64,
    pub logs: std::collections::VecDeque<LogEntry>,
    pub dock_tab: usize,

    // Diagnostics
    pub show_debug_overlay: bool,
    pub last_frame_ms: f32,
    pub last_event_backlog: usize,
    pub window_focused: bool,
    pub shader_disabled_reason: Option<String>,

    // Quota warnings
    pub quota_warnings: Vec<FxQuotaWarningEntry>,
    pub quota_focus_init: bool,
    pub bulk_confirm_modal: Option<FxBulkConfirmModalState>,

    // Selection stats cache
    pub selection_stats: bcvault_core::selection::SelectionStats,
    pub selection_stats_dirty: bool,
    pub selection_recount_calls: usize,
}

impl Default for FxState {
    fn default() -> Self {
        Self::new()
    }
}

impl FxState {
    pub fn resolved_theme(&self) -> FxTheme {
        self.theme_preview
            .unwrap_or_else(|| self.themes.resolve_theme(&self.prefs.theme))
    }

    pub fn resolved_target_theme(&self, hovered_swatch: Option<[u8; 3]>) -> FxTheme {
        if let Some(preview) = self.theme_preview {
            preview
        } else {
            self.themes
                .resolve_theme(&self.prefs.theme)
                .with_adaptive_accent(hovered_swatch)
        }
    }

    pub fn new() -> Self {
        let prefs = Prefs::default();
        let themes = ThemeRegistry::builtin_only();
        let active_theme = themes.resolve_theme(&prefs.theme);
        let now = Instant::now();
        Self {
            booted: false,
            schema_error: None,
            artists: Vec::new(),
            settings: Settings::default(),
            tex_cache: TextureCache::new(prefs.tex_budget_mb),
            prefs,
            themes,
            themes_dir: None,
            theme_preview: None,
            theme_editor: None,
            pending_theme_import_pick: false,
            theme_picked_import: std::sync::Arc::new(std::sync::Mutex::new(None)),
            pending_theme_export_pick: None,
            theme_picked_export: std::sync::Arc::new(std::sync::Mutex::new(None)),
            active_theme,
            motion: Motion::new(),
            palettes: HashMap::new(),
            cover_cache_bytes: 0,
            view_mode: ViewMode::Wall,
            prev_view_mode: ViewMode::Wall,
            view_switch_seq: 0,
            query: FxQuery::default(),
            nav: NavHistory::new(64),
            saved_views: Vec::new(),
            recents: Vec::new(),
            sidebar_collapsed: false,
            scroll_y: 0.0,
            pending_scroll_to_y: None,
            next_request_id: 1,
            active_wall_req_id: 0,
            active_facets_req_id: 0,
            active_stats_req_id: 0,
            active_search_req_id: 0,
            query_dirty_at: None,
            rows: Vec::new(),
            facets: Facets::default(),
            stats: Stats::default(),
            thumb_generation: 1,
            last_thumb_req_at: now,
            selected_keys: BTreeSet::new(),
            last_clicked_key: None,
            focused_key: None,
            hovered_key: None,
            hovered_art_id: None,
            marquee_start: None,
            marquee_current: None,
            last_card_rects: HashMap::new(),
            flip_origins: HashMap::new(),
            flip_seed_pending: false,
            flip_ghosts: Vec::new(),
            row_flashes: HashMap::new(),
            artist_dive_banner: None,
            drawer_key: None,
            drawer_opened_at: None,
            drawer_detail_requested: false,
            details_cache: HashMap::new(),
            drawer_slide_dir: 1.0,
            palette_open: false,
            palette_query: String::new(),
            palette_scope: SearchScope::All,
            palette_hits: Vec::new(),
            palette_sel: 0,
            focus_search_next_frame: false,
            focus_palette_next_frame: false,
            add_artist_input: String::new(),
            add_artist_pending: false,
            artist_filter: String::new(),
            sync_mode: SyncMode::Incremental,
            active_syncs: HashMap::new(),
            redeem_modal: None,
            quota_warnings: Vec::new(),
            quota_focus_init: false,
            bulk_confirm_modal: None,
            selection_stats: bcvault_core::selection::SelectionStats::default(),
            selection_stats_dirty: false,
            selection_recount_calls: 0,
            save_view_modal: None,
            settings_modal_open: false,
            settings_draft: None,
            settings_confirm_discard: false,
            settings_focus_first: false,
            settings_focus_keep_editing: false,
            settings_tab_anim_dir: 1.0,
            settings_tab_anim_gen: 0,
            settings_folders_filter: String::new(),
            settings_picked_out_dir: std::sync::Arc::new(std::sync::Mutex::new(None)),
            filename_pattern_input: String::new(),
            confirm_remove_artist: None,
            rename_artist_modal: None,
            link_folder_modal: None,
            reconcile_report_modal: None,
            resolve_missing_modal: None,
            show_resolve_missing_window: false,
            unavailable_modal: None,
            show_unavailable_window: false,
            folders: HashMap::new(),
            relink_picked_folder: std::sync::Arc::new(std::sync::Mutex::new(None)),
            pending_relink_pick: None,
            active_grouping: bcvault_core::groups::ActiveGrouping::None,
            group_sets: Vec::new(),
            groups: Vec::new(),
            builtin_groups: Vec::new(),
            collapsed_builtin_groups: BTreeSet::new(),
            pins: HashMap::new(),
            selected_artists: BTreeSet::new(),
            last_clicked_artist: None,
            group_assignment: bcvault_core::groups::Assignment::default(),
            sidebar_rows: vec![bcvault_core::groups::SidebarRow::AllArtists],
            sidebar_flip_active: HashSet::new(),
            sidebar_enter_active: HashSet::new(),
            sidebar_ghosts: Vec::new(),
            sidebar_anim_initialized: false,
            ungrouped_collapsed: false,
            sidebar_dirty: true,
            sidebar_last_filter: String::new(),
            sidebar_last_recompute_ts: 0,
            sidebar_recompute_count: 0,
            group_error: None,
            confirm_delete_group: None,
            confirm_delete_group_set: None,
            group_editor: None,
            group_set_modal: None,
            pending_group_pins: None,
            jobs: Vec::new(),
            speed_samples: std::collections::VecDeque::from(vec![0.0; 30]),
            last_speed_sample_at: now,
            completed_jobs_at: HashMap::new(),
            rate_limit: RateLimitStatus::Idle,
            toasts: Vec::new(),
            next_toast_id: 1,
            logs: std::collections::VecDeque::with_capacity(500),
            dock_tab: 0,
            show_debug_overlay: false,
            last_frame_ms: 0.0,
            last_event_backlog: 0,
            window_focused: true,
            shader_disabled_reason: None,
        }
    }

    pub fn card_size(&self) -> f32 {
        CARD_SIZES[self.prefs.card_size_idx.min(CARD_SIZES.len() - 1)]
    }

    pub fn unavailable_folder_artists(&self) -> Vec<i64> {
        self.folders
            .iter()
            .filter_map(|(&band_id, folders)| {
                if folders.iter().any(|f| f.state != FolderState::Ok) {
                    Some(band_id)
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn has_open_modal(&self) -> bool {
        self.confirm_remove_artist.is_some()
            || self.confirm_delete_group.is_some()
            || self.confirm_delete_group_set.is_some()
            || self.group_editor.is_some()
            || self.group_set_modal.is_some()
            || self.rename_artist_modal.is_some()
            || self.save_view_modal.is_some()
            || self.redeem_modal.is_some()
            || !self.quota_warnings.is_empty()
            || self.bulk_confirm_modal.is_some()
            || self.reconcile_report_modal.is_some()
            || (self.show_resolve_missing_window && self.resolve_missing_modal.is_some())
            || (self.show_unavailable_window && self.unavailable_modal.is_some())
            || self.link_folder_modal.is_some()
            || self.theme_editor.is_some()
            || self.settings_modal_open
    }

    pub fn close_top_modal(&mut self) -> bool {
        if self.theme_editor.take().is_some() {
            self.theme_preview = None;
            return true;
        }
        if !self.quota_warnings.is_empty() {
            self.quota_warnings.clear();
            self.quota_focus_init = false;
            return true;
        }
        if self.bulk_confirm_modal.take().is_some() {
            return true;
        }
        if self.confirm_remove_artist.take().is_some() {
            return true;
        }
        if self.confirm_delete_group.take().is_some() {
            return true;
        }
        if self.confirm_delete_group_set.take().is_some() {
            return true;
        }
        if self.group_editor.take().is_some() {
            return true;
        }
        if self.group_set_modal.take().is_some() {
            return true;
        }
        if self.rename_artist_modal.take().is_some() {
            return true;
        }
        if self.save_view_modal.take().is_some() {
            return true;
        }
        if self.redeem_modal.take().is_some() {
            return true;
        }
        if self.reconcile_report_modal.take().is_some() {
            return true;
        }
        if self.show_resolve_missing_window && self.resolve_missing_modal.is_some() {
            self.show_resolve_missing_window = false;
            return true;
        }
        if self.show_unavailable_window && self.unavailable_modal.is_some() {
            self.show_unavailable_window = false;
            return true;
        }
        if self.link_folder_modal.take().is_some() {
            return true;
        }
        if self.settings_modal_open {
            if let Some(ref draft) = self.settings_draft {
                match crate::overlays::settings::handle_settings_esc(
                    draft,
                    &self.settings,
                    &mut self.settings_confirm_discard,
                ) {
                    crate::overlays::settings::SettingsEscOutcome::Close => {
                        self.settings_modal_open = false;
                        self.settings_draft = None;
                    }
                    crate::overlays::settings::SettingsEscOutcome::ShowConfirm => {
                        self.settings_focus_keep_editing = true;
                    }
                    crate::overlays::settings::SettingsEscOutcome::KeepEditing => {}
                }
            } else {
                self.settings_modal_open = false;
                self.settings_confirm_discard = false;
            }
            return true;
        }
        false
    }

    pub fn open_settings_modal(&mut self) {
        self.settings_draft = Some(self.settings.clone());
        self.filename_pattern_input = self.settings.filename_pattern.clone();
        self.settings_confirm_discard = false;
        self.settings_focus_first = true;
        self.settings_focus_keep_editing = false;
        self.settings_modal_open = true;
    }

    pub fn current_view_state(&self) -> ViewState {
        ViewState {
            view: self.view_mode,
            query: self.query.clone(),
            scroll_y: self.scroll_y,
            focus: self.focused_key,
        }
    }

    pub fn mark_selection_dirty(&mut self) {
        self.selection_stats_dirty = true;
    }

    pub fn selection_stats(&mut self) -> bcvault_core::selection::SelectionStats {
        if self.selection_stats_dirty {
            self.selection_stats = bcvault_core::selection::compute_stats(
                self.rows.iter().map(|w| &w.rel),
                &self.selected_keys,
            );
            self.selection_stats_dirty = false;
            self.selection_recount_calls += 1;
        }
        self.selection_stats
    }

    pub fn push_quota_warning(&mut self, entry: FxQuotaWarningEntry) {
        if self.quota_warnings.is_empty() {
            self.quota_focus_init = true;
        }
        if let Some(existing) = self
            .quota_warnings
            .iter_mut()
            .find(|e| e.band_id == entry.band_id)
        {
            *existing = entry;
        } else {
            self.quota_warnings.push(entry);
        }
    }

    pub fn confirm_quota_warnings(&mut self, core_tx: &UnboundedSender<UiToWorkerCommand>) {
        if let Some(first) = self.quota_warnings.first() {
            let opts = first.opts.clone();
            let all_items: Vec<_> = self
                .quota_warnings
                .iter()
                .flat_map(|w| w.items.iter().copied())
                .collect();
            let _ = core_tx.send(UiToWorkerCommand::EnqueueDownloads {
                items: all_items,
                opts,
                ack_soft_quota: true,
                mode: bcvault_core::types::EnqueueMode::New,
            });
        }
        self.quota_warnings.clear();
        self.quota_focus_init = false;
    }

    pub fn library_remaining_count(&self) -> u32 {
        self.artists
            .iter()
            .map(|a| a.stats.eligible.saturating_sub(a.stats.eligible_downloaded))
            .sum()
    }

    pub fn ensure_sidebar_rows(&mut self, now_ts: bcvault_core::types::Ts) {
        self.ensure_sidebar_rows_at(now_ts, 0.0);
    }

    pub fn ensure_sidebar_rows_at(&mut self, now_ts: bcvault_core::types::Ts, now_s: f64) {
        let filter_changed = self.artist_filter != self.sidebar_last_filter;
        let age_refresh = matches!(
            self.active_grouping,
            bcvault_core::groups::ActiveGrouping::Builtin(
                bcvault_core::groups::BuiltinGrouping::SyncFreshness
            )
        ) && now_ts.saturating_sub(self.sidebar_last_recompute_ts) >= 60;
        if !self.sidebar_dirty && !filter_changed && !age_refresh {
            return;
        }
        let prev_rows = std::mem::take(&mut self.sidebar_rows);
        match self.active_grouping {
            bcvault_core::groups::ActiveGrouping::None => {
                self.builtin_groups.clear();
                self.group_assignment = bcvault_core::groups::Assignment {
                    groups: Vec::new(),
                    ungrouped: self.artists.iter().map(|a| a.band_id).collect(),
                };
            }
            bcvault_core::groups::ActiveGrouping::Builtin(kind) => {
                let (bg, assign) = bcvault_core::groups::assign_builtin(
                    kind,
                    &self.artists,
                    &self.folders,
                    now_ts,
                    &self.collapsed_builtin_groups,
                );
                self.builtin_groups = bg;
                self.group_assignment = assign;
            }
            bcvault_core::groups::ActiveGrouping::CustomSet(_) => {
                self.builtin_groups.clear();
                self.group_assignment =
                    bcvault_core::groups::assign_custom(&self.artists, &self.groups, &self.pins);
            }
        }
        let artists_by_id: HashMap<i64, usize> = self
            .artists
            .iter()
            .enumerate()
            .map(|(i, a)| (a.band_id, i))
            .collect();
        let active_groups: &[bcvault_core::groups::ArtistGroup] = match self.active_grouping {
            bcvault_core::groups::ActiveGrouping::Builtin(_) => &self.builtin_groups,
            bcvault_core::groups::ActiveGrouping::CustomSet(_) => &self.groups,
            bcvault_core::groups::ActiveGrouping::None => &[],
        };
        let new_rows = bcvault_core::groups::build_sidebar_rows(
            &self.group_assignment,
            active_groups,
            &artists_by_id,
            &self.artists,
            &self.artist_filter,
            self.ungrouped_collapsed,
        );

        let should_animate = self.sidebar_anim_initialized && !self.motion.is_instant();
        if should_animate {
            const ROW_H: f32 = 28.0;
            let mut old_pos: HashMap<SidebarRowKey, usize> =
                HashMap::with_capacity(prev_rows.len());
            for (idx, row) in prev_rows.iter().enumerate() {
                old_pos.insert(SidebarRowKey::from_row(row), idx);
            }
            let mut new_keys: HashSet<SidebarRowKey> = HashSet::with_capacity(new_rows.len());
            let mut enter_idx = 0usize;

            for (new_idx, row) in new_rows.iter().enumerate() {
                let key = SidebarRowKey::from_row(row);
                new_keys.insert(key);
                let target_y = new_idx as f32 * ROW_H;
                if let Some(&old_idx) = old_pos.get(&key) {
                    if old_idx != new_idx {
                        if !self.sidebar_flip_active.contains(&key) {
                            let old_y = old_idx as f32 * ROW_H;
                            let dy = (old_y - target_y).clamp(-320.0, 320.0);
                            self.motion
                                .set_vec2(("sb_flip", key), egui::Vec2::new(0.0, target_y + dy));
                        }
                        self.sidebar_flip_active.insert(key);
                    }
                } else {
                    let stagger_x = ((enter_idx.min(10) as f32) * 1.5) + 12.0;
                    let start = match key {
                        SidebarRowKey::GroupHeader(_) | SidebarRowKey::UngroupedHeader => {
                            egui::Vec2::new(-stagger_x - 6.0, (target_y - 6.0).max(0.0))
                        }
                        _ => egui::Vec2::new(-stagger_x, (target_y - 14.0).max(0.0)),
                    };
                    self.motion.set_vec2(("sb_flip", key), start);
                    self.motion.set_val(("sb_enter", key), 0.0);
                    self.sidebar_flip_active.insert(key);
                    self.sidebar_enter_active.insert(key);
                    enter_idx += 1;
                }
            }

            self.sidebar_ghosts.clear();
            for (old_idx, row) in prev_rows.iter().enumerate() {
                if self.sidebar_ghosts.len() >= 32 {
                    break;
                }
                let key = SidebarRowKey::from_row(row);
                if !new_keys.contains(&key) {
                    let (label, is_header, band_id) = match row {
                        bcvault_core::groups::SidebarRow::AllArtists => {
                            ("All artists".to_string(), false, Some(0))
                        }
                        bcvault_core::groups::SidebarRow::GroupHeader { name, .. } => {
                            (name.clone(), true, None)
                        }
                        bcvault_core::groups::SidebarRow::UngroupedHeader { .. } => {
                            ("Ungrouped".to_string(), true, None)
                        }
                        bcvault_core::groups::SidebarRow::Artist { band_id } => {
                            let name = artists_by_id
                                .get(band_id)
                                .and_then(|&i| self.artists.get(i))
                                .map(|a| a.display_name().to_string())
                                .unwrap_or_default();
                            (name, false, Some(*band_id))
                        }
                    };
                    self.sidebar_ghosts.push(SidebarGhost {
                        key,
                        label,
                        is_header,
                        band_id,
                        y_rel: old_idx as f32 * ROW_H,
                        created_s: now_s,
                    });
                }
            }
        } else {
            self.sidebar_flip_active.clear();
            self.sidebar_enter_active.clear();
            self.sidebar_ghosts.clear();
        }

        if !self.artists.is_empty() {
            self.sidebar_anim_initialized = true;
        }
        self.sidebar_rows = new_rows;
        self.sidebar_last_filter = self.artist_filter.clone();
        self.sidebar_last_recompute_ts = now_ts;
        self.sidebar_dirty = false;
        self.sidebar_recompute_count += 1;
    }

    pub fn group_member_ids(&self, group_id: i64) -> Vec<i64> {
        self.group_assignment
            .groups
            .iter()
            .find(|g| g.group_id == group_id)
            .map(|g| g.band_ids.clone())
            .unwrap_or_default()
    }

    pub fn trigger_download_remaining_artists(
        &mut self,
        title: String,
        ids: Vec<i64>,
        core_tx: &UnboundedSender<UiToWorkerCommand>,
    ) {
        let opts = DownloadOpts {
            format: self.settings.format,
            out_dir: self.settings.out_dir.clone(),
            batch_limit: self.settings.batch_limit,
        };
        let per_artist: Vec<(i64, u32, u32)> = ids
            .iter()
            .filter_map(|&bid| {
                self.artists.iter().find(|a| a.band_id == bid).map(|a| {
                    let rem = a.stats.eligible.saturating_sub(a.stats.eligible_downloaded);
                    (bid, rem, a.stats.quota_used_30d)
                })
            })
            .filter(|&(_, rem, _)| rem > 0)
            .collect();
        if per_artist.is_empty() {
            return;
        }
        let is_multi = ids.len() > 1;
        let preview = bcvault_core::worker::quota::preview_bulk(
            &per_artist,
            self.settings.batch_limit,
            self.settings.quota_soft,
            self.settings.quota_hard,
            false,
        );
        let action = FxBulkAction::Remaining(bcvault_core::types::RemainingScope::Artists(ids));
        if bcvault_core::worker::quota::should_confirm_bulk(is_multi, &preview) {
            let mut rows = Vec::with_capacity(preview.len());
            let mut total_new = 0u32;
            let mut total_queue = 0u32;
            let mut total_deferred = 0u32;
            for p in preview {
                let artist_name = self
                    .artists
                    .iter()
                    .find(|a| a.band_id == p.band_id)
                    .map(|a| a.display_name().to_string())
                    .unwrap_or_else(|| format!("Artist {}", p.band_id));
                total_new = total_new.saturating_add(p.requested);
                total_queue = total_queue.saturating_add(p.allowed);
                total_deferred = total_deferred.saturating_add(p.deferred);
                rows.push(FxBulkConfirmRow {
                    artist_name,
                    preview: p,
                });
            }
            self.bulk_confirm_modal = Some(FxBulkConfirmModalState {
                title,
                rows,
                total_new,
                total_queue,
                total_deferred,
                action,
                focus_init: true,
            });
        } else if let FxBulkAction::Remaining(scope) = action {
            let _ = core_tx.send(UiToWorkerCommand::EnqueueRemaining {
                scope,
                opts,
                ack_soft_quota: false,
            });
        }
    }

    pub fn trigger_download_remaining(
        &mut self,
        target: RemainingTarget,
        core_tx: &UnboundedSender<UiToWorkerCommand>,
    ) {
        let opts = DownloadOpts {
            format: self.settings.format,
            out_dir: self.settings.out_dir.clone(),
            batch_limit: self.settings.batch_limit,
        };
        let (title, is_library_or_multi, per_artist, action) = match target {
            RemainingTarget::View => {
                let mut counts: HashMap<i64, u32> = HashMap::new();
                let mut order: Vec<i64> = Vec::new();
                let mut keys: Vec<ItemKey> = Vec::new();
                for wr in &self.rows {
                    if wr.rel.is_download_candidate() {
                        keys.push(wr.rel.key);
                        if !counts.contains_key(&wr.rel.band_id) {
                            order.push(wr.rel.band_id);
                        }
                        *counts.entry(wr.rel.band_id).or_insert(0) += 1;
                    }
                }
                if keys.is_empty() {
                    return;
                }
                let per_artist: Vec<(i64, u32, u32)> = order
                    .into_iter()
                    .map(|bid| {
                        let req = counts.get(&bid).copied().unwrap_or(0);
                        let used = self
                            .artists
                            .iter()
                            .find(|a| a.band_id == bid)
                            .map(|a| a.stats.quota_used_30d)
                            .unwrap_or(0);
                        (bid, req, used)
                    })
                    .collect();
                (
                    "Download Remaining in View".to_string(),
                    per_artist.len() > 1,
                    per_artist,
                    FxBulkAction::View(keys),
                )
            }
            RemainingTarget::ThisArtist => {
                let Some(bid) = self.query.base.band_id else {
                    return;
                };
                let per_artist: Vec<(i64, u32, u32)> = self
                    .artists
                    .iter()
                    .find(|a| a.band_id == bid)
                    .map(|a| {
                        let rem = a.stats.eligible.saturating_sub(a.stats.eligible_downloaded);
                        (bid, rem, a.stats.quota_used_30d)
                    })
                    .filter(|&(_, rem, _)| rem > 0)
                    .into_iter()
                    .collect();
                if per_artist.is_empty() {
                    return;
                }
                (
                    "Download Remaining for Artist".to_string(),
                    false,
                    per_artist,
                    FxBulkAction::Remaining(bcvault_core::types::RemainingScope::Artists(vec![
                        bid,
                    ])),
                )
            }
            RemainingTarget::AllArtists => {
                let per_artist: Vec<(i64, u32, u32)> = self
                    .artists
                    .iter()
                    .filter_map(|a| {
                        let rem = a.stats.eligible.saturating_sub(a.stats.eligible_downloaded);
                        if rem > 0 {
                            Some((a.band_id, rem, a.stats.quota_used_30d))
                        } else {
                            None
                        }
                    })
                    .collect();
                if per_artist.is_empty() {
                    return;
                }
                (
                    "Download Remaining for All Artists".to_string(),
                    true,
                    per_artist,
                    FxBulkAction::Remaining(bcvault_core::types::RemainingScope::Library),
                )
            }
        };

        let preview = bcvault_core::worker::quota::preview_bulk(
            &per_artist,
            self.settings.batch_limit,
            self.settings.quota_soft,
            self.settings.quota_hard,
            false,
        );

        if bcvault_core::worker::quota::should_confirm_bulk(is_library_or_multi, &preview) {
            let mut rows = Vec::with_capacity(preview.len());
            let mut total_new = 0u32;
            let mut total_queue = 0u32;
            let mut total_deferred = 0u32;
            for p in preview {
                let artist_name = self
                    .artists
                    .iter()
                    .find(|a| a.band_id == p.band_id)
                    .map(|a| a.display_name().to_string())
                    .unwrap_or_else(|| format!("Artist {}", p.band_id));
                total_new = total_new.saturating_add(p.requested);
                total_queue = total_queue.saturating_add(p.allowed);
                total_deferred = total_deferred.saturating_add(p.deferred);
                rows.push(FxBulkConfirmRow {
                    artist_name,
                    preview: p,
                });
            }
            self.bulk_confirm_modal = Some(FxBulkConfirmModalState {
                title,
                rows,
                total_new,
                total_queue,
                total_deferred,
                action,
                focus_init: false,
            });
        } else {
            Self::send_bulk_action(action, opts, false, core_tx);
        }
    }

    pub fn confirm_bulk_modal(&mut self, core_tx: &UnboundedSender<UiToWorkerCommand>) {
        if let Some(modal) = self.bulk_confirm_modal.take() {
            let opts = DownloadOpts {
                format: self.settings.format,
                out_dir: self.settings.out_dir.clone(),
                batch_limit: self.settings.batch_limit,
            };
            Self::send_bulk_action(modal.action, opts, true, core_tx);
        }
    }

    fn send_bulk_action(
        action: FxBulkAction,
        opts: DownloadOpts,
        ack_soft_quota: bool,
        core_tx: &UnboundedSender<UiToWorkerCommand>,
    ) {
        match action {
            FxBulkAction::View(items) => {
                let _ = core_tx.send(UiToWorkerCommand::EnqueueDownloads {
                    items,
                    opts,
                    ack_soft_quota,
                    mode: bcvault_core::types::EnqueueMode::New,
                });
            }
            FxBulkAction::Remaining(scope) => {
                let _ = core_tx.send(UiToWorkerCommand::EnqueueRemaining {
                    scope,
                    opts,
                    ack_soft_quota,
                });
            }
        }
    }

    pub fn push_toast(&mut self, kind: ToastKind, message: impl Into<String>, now_s: f64) {
        self.push_toast_with_undo(kind, message, now_s, None);
    }

    pub fn push_toast_with_undo(
        &mut self,
        kind: ToastKind,
        message: impl Into<String>,
        now_s: f64,
        undo_batch_id: Option<u64>,
    ) {
        let id = self.next_toast_id;
        self.next_toast_id += 1;
        self.toasts.push(Toast {
            id,
            kind,
            message: message.into(),
            created_s: now_s,
            undo_batch_id,
        });
        while self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    pub fn snapshot_for_flip(&mut self, now_s: f64) {
        self.flip_origins.clear();
        for (k, r) in &self.last_card_rects {
            if self.flip_origins.len() < 500 {
                self.flip_origins.insert(*k, *r);
            }
        }
        self.flip_seed_pending = !self.last_card_rects.is_empty();
        self.flip_ghosts.clear();
        for row in &self.rows {
            if self.flip_ghosts.len() >= 40 {
                break;
            }
            if let Some(&r) = self.last_card_rects.get(&row.rel.key) {
                self.flip_ghosts.push((row.rel.key, row.art_id, r, now_s));
            }
        }
    }

    pub fn mark_query_immediate(&mut self) {
        self.query_dirty_at = Some(
            Instant::now()
                .checked_sub(Duration::from_secs(1))
                .unwrap_or_else(Instant::now),
        );
    }

    pub fn mark_search_debounced(&mut self) {
        self.query_dirty_at = Some(Instant::now());
    }

    pub fn flush_debounced_queries(&mut self, fx_tx: &UnboundedSender<FxCommand>) {
        if let Some(dirty_at) = self.query_dirty_at
            && dirty_at.elapsed() >= Duration::from_millis(120)
        {
            self.query_dirty_at = None;
            self.issue_wall_and_facets_query(fx_tx);
        }
    }

    pub fn issue_wall_and_facets_query(&mut self, fx_tx: &UnboundedSender<FxCommand>) {
        let wall_req = self.next_request_id;
        self.next_request_id += 1;
        self.active_wall_req_id = wall_req;
        self.thumb_generation = self.thumb_generation.wrapping_add(1);

        let facets_req = self.next_request_id;
        self.next_request_id += 1;
        self.active_facets_req_id = facets_req;

        let _ = fx_tx.send(FxCommand::QueryWall {
            request_id: wall_req,
            query: self.query.clone(),
        });
        let _ = fx_tx.send(FxCommand::QueryFacets {
            request_id: facets_req,
            query: self.query.clone(),
        });

        if self.view_mode == ViewMode::Stats {
            let stats_req = self.next_request_id;
            self.next_request_id += 1;
            self.active_stats_req_id = stats_req;
            let _ = fx_tx.send(FxCommand::QueryStats {
                request_id: stats_req,
            });
        }
    }

    pub fn check_lazy_drawer_detail(&mut self, fx_tx: &UnboundedSender<FxCommand>) {
        if let (Some(key), Some(opened_at)) = (self.drawer_key, self.drawer_opened_at)
            && !self.drawer_detail_requested
            && opened_at.elapsed() >= Duration::from_millis(400)
        {
            self.drawer_detail_requested = true;
            if !self.details_cache.contains_key(&key) {
                let _ = fx_tx.send(FxCommand::EnsureDetail { key });
            }
        }
    }

    pub fn sample_download_speed(&mut self) {
        if self.last_speed_sample_at.elapsed() >= Duration::from_millis(950) {
            self.last_speed_sample_at = Instant::now();
            let total_bps: f64 = self.jobs.iter().map(|j| j.speed_bps).sum();
            if self.speed_samples.len() >= 30 {
                self.speed_samples.pop_front();
            }
            self.speed_samples.push_back(total_bps);
        }
    }

    pub fn apply_core_event(
        &mut self,
        ev: WorkerToUiEvent,
        fx_tx: &UnboundedSender<FxCommand>,
        now_s: f64,
    ) {
        match ev {
            WorkerToUiEvent::Ready {
                artists,
                settings,
                folders,
                active_grouping,
                group_sets,
                groups,
                pins,
            } => {
                self.artists = artists;
                self.settings = settings;
                self.folders.clear();
                for f in folders {
                    self.folders.entry(f.band_id).or_default().push(f);
                }
                self.active_grouping = active_grouping;
                self.group_sets = group_sets;
                self.groups = groups;
                self.pins = pins;
                self.sidebar_dirty = true;
                self.booted = true;
                self.issue_wall_and_facets_query(fx_tx);
            }
            WorkerToUiEvent::ArtistUpserted(av) => {
                self.add_artist_pending = false;
                if let Some(slot) = self.artists.iter_mut().find(|a| a.band_id == av.band_id) {
                    *slot = av;
                } else {
                    self.artists.push(av);
                    self.artists.sort_by(|a, b| {
                        a.display_name()
                            .to_lowercase()
                            .cmp(&b.display_name().to_lowercase())
                    });
                }
                self.sidebar_dirty = true;
            }
            WorkerToUiEvent::ArtistRemoved { band_id } => {
                self.artists.retain(|a| a.band_id != band_id);
                if self.query.base.band_id == Some(band_id) {
                    self.query.base.band_id = None;
                }
                self.sidebar_dirty = true;
                self.issue_wall_and_facets_query(fx_tx);
            }
            WorkerToUiEvent::ArtistStats { band_id, stats } => {
                if let Some(a) = self.artists.iter_mut().find(|a| a.band_id == band_id) {
                    a.stats = stats;
                }
                self.sidebar_dirty = true;
                self.issue_wall_and_facets_query(fx_tx);
            }
            WorkerToUiEvent::SyncProgress(prog) => {
                self.active_syncs.insert(prog.band_id, prog);
            }
            WorkerToUiEvent::SyncFinished {
                band_id,
                new_items,
                updated,
                error,
                ..
            } => {
                self.add_artist_pending = false;
                self.active_syncs.remove(&band_id);
                if let Some(err) = error {
                    self.push_toast(ToastKind::Error, format!("Sync failed: {err}"), now_s);
                } else if new_items > 0 || updated > 0 {
                    self.push_toast(
                        ToastKind::Success,
                        format!("Synced +{new_items} new, {updated} updated"),
                        now_s,
                    );
                }
                self.issue_wall_and_facets_query(fx_tx);
            }
            WorkerToUiEvent::ReleasesLoaded { .. }
            | WorkerToUiEvent::ThumbReady { .. }
            | WorkerToUiEvent::QuotaStatus { .. }
            | WorkerToUiEvent::LibraryProgress { .. } => {}
            WorkerToUiEvent::QuotaWarning {
                band_id,
                used_30d,
                requested,
                soft,
                hard,
                items,
                opts,
            } => {
                self.push_quota_warning(FxQuotaWarningEntry {
                    band_id,
                    used_30d,
                    requested,
                    soft,
                    hard,
                    items,
                    opts,
                });
            }
            WorkerToUiEvent::EnqueueSummary(summary) => {
                let msg = summary.format_summary();
                let kind = if summary.deferred.is_empty() {
                    ToastKind::Info
                } else {
                    ToastKind::Warn
                };
                self.push_toast(kind, msg.clone(), now_s);
                for d in &summary.deferred {
                    let cause = match d.cause {
                        bcvault_core::worker::messages::DeferCause::BatchLimit => "batch limit",
                        bcvault_core::worker::messages::DeferCause::HardCap => "hard cap",
                        bcvault_core::worker::messages::DeferCause::Blocked30Min => "30-min block",
                    };
                    self.logs.push_back(LogEntry {
                        ts: 0,
                        level: bcvault_core::types::LogLevel::Warn,
                        scope: "quota",
                        message: format!(
                            "Deferred {} release(s) for artist {} ({cause})",
                            d.count, d.band_id
                        ),
                        item: None,
                    });
                    if self.logs.len() > 2000 {
                        self.logs.pop_front();
                    }
                }
            }
            WorkerToUiEvent::ReconcileReport { report } => {
                self.reconcile_report_modal = Some(FxReconcileReportModalState::new(report));
            }
            WorkerToUiEvent::FolderStatus { band_id, folders } => {
                if folders.is_empty() {
                    self.folders.remove(&band_id);
                } else {
                    self.folders.insert(band_id, folders);
                }
                self.sidebar_dirty = true;
            }
            WorkerToUiEvent::LibraryApplied { batch_id, summary } => {
                let mut parts = Vec::new();
                if summary.adopted > 0 {
                    parts.push(format!("{} adopted", summary.adopted));
                }
                if summary.relinked > 0 {
                    parts.push(format!("{} relinked", summary.relinked));
                }
                if summary.archived > 0 {
                    parts.push(format!("{} archived", summary.archived));
                }
                if summary.restored > 0 {
                    parts.push(format!("{} restored", summary.restored));
                }
                if summary.ignored > 0 {
                    parts.push(format!("{} ignored", summary.ignored));
                }
                if summary.unmarked > 0 {
                    parts.push(format!("{} unmarked", summary.unmarked));
                }
                let summary_str = if parts.is_empty() {
                    "Library changes applied".to_string()
                } else {
                    parts.join(", ")
                };
                self.push_toast_with_undo(
                    ToastKind::Info,
                    summary_str,
                    now_s,
                    if batch_id > 0 { Some(batch_id) } else { None },
                );
                self.issue_wall_and_facets_query(fx_tx);
            }
            WorkerToUiEvent::MissingFound {
                batch_id,
                items,
                mass_missing,
            } => {
                if !items.is_empty() {
                    self.resolve_missing_modal = Some(FxResolveMissingModalState::new(
                        batch_id,
                        items,
                        mass_missing,
                        self.settings.deleted_policy,
                    ));
                    self.show_resolve_missing_window = true;
                } else {
                    self.resolve_missing_modal = None;
                    self.show_resolve_missing_window = false;
                }
            }
            WorkerToUiEvent::ReleaseUpdated(rel) => {
                let key = rel.key;
                let was_dl = self
                    .rows
                    .iter()
                    .find(|w| w.rel.key == key)
                    .is_some_and(|w| w.rel.downloaded);
                for w in &mut self.rows {
                    if w.rel.key == key {
                        w.rel = (*rel).clone();
                    }
                }
                self.selection_stats_dirty = true;
                self.row_flashes.insert(key, now_s + 0.6);
                if !was_dl && rel.downloaded {
                    let _ = self
                        .motion
                        .val_from(("dl_check", key), 0.0, 1.0, SpringCfg::POP);
                }
                let facets_req = self.next_request_id;
                self.next_request_id += 1;
                self.active_facets_req_id = facets_req;
                let _ = fx_tx.send(FxCommand::QueryFacets {
                    request_id: facets_req,
                    query: self.query.clone(),
                });
            }
            WorkerToUiEvent::JobQueued(jv) => {
                if let Some(slot) = self.jobs.iter_mut().find(|j| j.view.id == jv.id) {
                    slot.view = jv;
                } else {
                    self.jobs.push(FxJob {
                        view: jv,
                        bytes_done: 0,
                        bytes_total: None,
                        speed_bps: 0.0,
                    });
                }
            }
            WorkerToUiEvent::JobState { job_id, state } => {
                if matches!(state, JobState::Done { .. })
                    && !self.completed_jobs_at.contains_key(&job_id)
                {
                    self.completed_jobs_at.insert(job_id, now_s);
                }
                if let Some(slot) = self.jobs.iter_mut().find(|j| j.view.id == job_id) {
                    if !matches!(state, JobState::Downloading) {
                        slot.speed_bps = 0.0;
                    }
                    slot.view.state = state;
                }
            }
            WorkerToUiEvent::DownloadByteProgress {
                job_id,
                bytes,
                total,
                bytes_per_sec,
            } => {
                if let Some(slot) = self.jobs.iter_mut().find(|j| j.view.id == job_id) {
                    slot.bytes_done = bytes;
                    slot.bytes_total = total;
                    slot.speed_bps = bytes_per_sec;
                }
            }
            WorkerToUiEvent::RateLimit(st) => {
                self.rate_limit = st;
            }
            WorkerToUiEvent::SettingsUpdated(st) => {
                self.settings = st;
                self.push_toast(ToastKind::Info, "Settings saved", now_s);
            }
            WorkerToUiEvent::FolderUnavailable {
                batch_id,
                folders,
                prompt,
            } => {
                if prompt && !folders.is_empty() {
                    self.unavailable_modal = Some(FxUnavailableModalState::new(
                        batch_id,
                        folders,
                        self.settings.deleted_policy,
                    ));
                    self.show_unavailable_window = true;
                }
            }
            WorkerToUiEvent::ArtistGroupsChanged {
                active_grouping,
                group_sets,
                groups,
                pins,
            } => {
                self.active_grouping = active_grouping;
                self.group_sets = group_sets;
                self.groups = groups;
                self.pins = pins;
                self.group_error = None;
                self.sidebar_dirty = true;
            }
            WorkerToUiEvent::ArtistGroupError { message } => {
                self.push_toast(ToastKind::Error, message.clone(), now_s);
                self.group_error = Some(message);
            }
            WorkerToUiEvent::Log(entry) => {
                if entry.level == bcvault_core::types::LogLevel::Error {
                    self.push_toast(ToastKind::Error, entry.message.clone(), now_s);
                }
                if self.logs.len() >= 500 {
                    self.logs.pop_front();
                }
                self.logs.push_back(entry);
            }
        }
    }

    pub fn apply_fx_event(&mut self, ev: FxEvent, now_s: f64) {
        match ev {
            FxEvent::Ready {
                prefs,
                saved_views,
                palettes,
                recents,
                schema_error,
                cover_cache_bytes,
            } => {
                self.motion.level = prefs.effects;
                self.motion.reduced_motion = prefs.reduced_motion;
                self.view_mode = prefs.view;
                self.prev_view_mode = prefs.view;
                self.tex_cache.set_budget_mb(prefs.tex_budget_mb);
                self.prefs = prefs;
                self.saved_views = saved_views;
                self.recents = recents;
                self.schema_error = schema_error;
                self.cover_cache_bytes = cover_cache_bytes;
                for p in palettes {
                    self.palettes.insert(p.art_id, p);
                }
            }
            FxEvent::WallLoaded {
                request_id,
                chunk,
                last: _,
                rows,
            } => {
                if request_id != self.active_wall_req_id {
                    return;
                }
                if chunk == 0 {
                    if !self.rows.is_empty() {
                        self.snapshot_for_flip(now_s);
                    }
                    self.rows = rows;
                    self.selection_stats_dirty = true;
                } else {
                    self.rows.extend(rows);
                }
                if self.focused_key.is_none() {
                    self.focused_key = self.rows.first().map(|r| r.rel.key);
                }
            }
            FxEvent::FacetsLoaded { request_id, facets } => {
                if request_id == self.active_facets_req_id {
                    self.facets = facets;
                }
            }
            FxEvent::StatsLoaded { request_id, stats } => {
                if request_id == self.active_stats_req_id {
                    self.stats = stats;
                }
            }
            FxEvent::SearchResults { request_id, hits } => {
                if request_id == self.active_search_req_id {
                    self.palette_hits = hits;
                    self.palette_sel = 0;
                }
            }
            FxEvent::ThumbReady {
                art_id,
                tier,
                image,
            } => {
                self.tex_cache.enqueue_upload(art_id, tier, image);
            }
            FxEvent::PaletteReady { art_id, palette } => {
                for w in &mut self.rows {
                    if w.art_id == Some(art_id) {
                        w.hue = palette.hue;
                        w.accent = Some(palette.accent);
                    }
                }
                self.palettes.insert(art_id, palette);
            }
            FxEvent::DetailLoaded { key, detail } => {
                self.details_cache.insert(key, *detail);
            }
            FxEvent::SavedViewsUpdated(views) => {
                self.saved_views = views;
            }
            FxEvent::CoverCachePurged => {
                self.tex_cache.clear();
                self.cover_cache_bytes = 0;
            }
            FxEvent::ThumbFailed { .. } => {}
            FxEvent::Notice { kind, message } => {
                self.push_toast(kind, message, now_s);
            }
        }
    }

    pub fn dispatch(
        &mut self,
        action: FxAction,
        core_tx: &UnboundedSender<UiToWorkerCommand>,
        fx_tx: &UnboundedSender<FxCommand>,
        now_s: f64,
    ) {
        match action {
            FxAction::SetView(vm) => {
                if self.view_mode != vm {
                    self.nav.push(self.current_view_state());
                    self.prev_view_mode = self.view_mode;
                    self.view_mode = vm;
                    self.view_switch_seq = self.view_switch_seq.wrapping_add(1);
                    let _ = fx_tx.send(FxCommand::SetPref {
                        key: "view".to_string(),
                        value: vm.as_str().to_string(),
                    });
                    if vm == ViewMode::Stats {
                        let req = self.next_request_id;
                        self.next_request_id += 1;
                        self.active_stats_req_id = req;
                        let _ = fx_tx.send(FxCommand::QueryStats { request_id: req });
                    }
                }
            }
            FxAction::OpenPalette => {
                self.palette_open = true;
                self.palette_query.clear();
                self.palette_scope = SearchScope::All;
                self.focus_palette_next_frame = true;
                let req = self.next_request_id;
                self.next_request_id += 1;
                self.active_search_req_id = req;
                let _ = fx_tx.send(FxCommand::Search {
                    request_id: req,
                    text: String::new(),
                    scope: SearchScope::All,
                });
            }
            FxAction::ClosePalette => {
                self.palette_open = false;
            }
            FxAction::FocusSearch => {
                self.focus_search_next_frame = true;
            }
            FxAction::SetFilter(f) => {
                if self.query.base.filter != f || self.query.smart.is_some() {
                    self.nav.push(self.current_view_state());
                    self.query.smart = None;
                    self.query.base.filter = f;
                    self.issue_wall_and_facets_query(fx_tx);
                }
            }
            FxAction::SetSort(sort, dir) => {
                self.nav.push(self.current_view_state());
                self.query.fx_sort = sort;
                self.query.base.dir = dir;
                self.issue_wall_and_facets_query(fx_tx);
            }
            FxAction::SetGroup(g) => {
                if self.query.group != g {
                    self.nav.push(self.current_view_state());
                    self.snapshot_for_flip(now_s);
                    self.query.group = g;
                }
            }
            FxAction::SetHue(hue) => {
                if self.query.hue != hue {
                    self.nav.push(self.current_view_state());
                    self.query.hue = hue;
                    self.issue_wall_and_facets_query(fx_tx);
                }
            }
            FxAction::SelectSmartView(sv) => {
                self.nav.push(self.current_view_state());
                self.query.smart = sv;
                self.issue_wall_and_facets_query(fx_tx);
            }
            FxAction::SelectArtist(band_id) => {
                self.selected_artists.clear();
                if let Some(bid) = band_id {
                    self.selected_artists.insert(bid);
                    self.last_clicked_artist = Some(bid);
                } else {
                    self.last_clicked_artist = None;
                }
                if self.query.base.band_id != band_id || !self.query.base.band_ids.is_empty() {
                    self.nav.push(self.current_view_state());
                    self.query.base.band_id = band_id;
                    self.query.base.band_ids.clear();
                    if let Some(bid) = band_id {
                        let _ = fx_tx.send(FxCommand::TouchRecent {
                            kind: "artist".to_string(),
                            r#ref: bid.to_string(),
                        });
                    }
                    self.issue_wall_and_facets_query(fx_tx);
                }
            }
            FxAction::SelectArtists(ids) => {
                self.selected_artists = ids.iter().copied().collect();
                self.nav.push(self.current_view_state());
                if ids.is_empty() {
                    self.query.base.band_id = None;
                    self.query.base.band_ids.clear();
                } else if ids.len() == 1 {
                    self.query.base.band_id = Some(ids[0]);
                    self.query.base.band_ids.clear();
                } else {
                    self.query.base.band_id = None;
                    self.query.base.band_ids = ids;
                }
                self.issue_wall_and_facets_query(fx_tx);
            }
            FxAction::OpenDrawer(key) => {
                self.drawer_key = Some(key);
                self.drawer_opened_at = Some(Instant::now());
                self.drawer_detail_requested = false;
                if let Some(w) = self.rows.iter().find(|r| r.rel.key == key)
                    && let Some(art_id) = w.art_id
                {
                    let _ = fx_tx.send(FxCommand::EnsureHero { art_id });
                }
            }
            FxAction::CloseDrawer => {
                self.drawer_key = None;
                self.drawer_opened_at = None;
                self.drawer_detail_requested = false;
            }
            FxAction::DrawerStep(delta) => {
                if let Some(curr) = self.drawer_key
                    && let Some(pos) = self.rows.iter().position(|r| r.rel.key == curr)
                {
                    let len = self.rows.len() as isize;
                    let next_idx =
                        (pos as isize + delta as isize).clamp(0, (len - 1).max(0)) as usize;
                    if let Some(next_row) = self.rows.get(next_idx) {
                        let next_key = next_row.rel.key;
                        if next_key != curr {
                            self.drawer_slide_dir = if delta >= 0 { 1.0 } else { -1.0 };
                            self.focused_key = Some(next_key);
                            self.dispatch(FxAction::OpenDrawer(next_key), core_tx, fx_tx, now_s);
                        }
                    }
                }
            }
            FxAction::ToggleSelect(key) => {
                if !self.selected_keys.remove(&key) {
                    self.selected_keys.insert(key);
                    let _ = self
                        .motion
                        .val_from(("sel_pop", key), 0.5, 1.0, SpringCfg::POP);
                }
                self.last_clicked_key = Some(key);
                self.focused_key = Some(key);
                self.selection_stats_dirty = true;
            }
            FxAction::SelectRange(key) => {
                let anchor = self.last_clicked_key.or(self.focused_key).unwrap_or(key);
                let refs: Vec<&bcvault_core::types::ReleaseRow> =
                    self.rows.iter().map(|w| &w.rel).collect();
                let range = bcvault_core::selection::range_keys(
                    &refs,
                    anchor,
                    key,
                    bcvault_core::selection::SelectScope::All,
                );
                self.selected_keys.extend(range);
                self.focused_key = Some(key);
                self.selection_stats_dirty = true;
            }
            FxAction::SelectAll => {
                self.selected_keys = bcvault_core::selection::select(
                    self.rows.iter().map(|w| &w.rel),
                    bcvault_core::selection::SelectScope::All,
                );
                self.selection_stats_dirty = true;
            }
            FxAction::InvertSelection => {
                self.selected_keys = bcvault_core::selection::invert(
                    self.rows.iter().map(|w| &w.rel),
                    &self.selected_keys,
                    bcvault_core::selection::SelectScope::All,
                );
                self.selection_stats_dirty = true;
            }
            FxAction::SelectRemaining => {
                self.selected_keys = bcvault_core::selection::select(
                    self.rows.iter().map(|w| &w.rel),
                    bcvault_core::selection::SelectScope::Remaining,
                );
                self.selection_stats_dirty = true;
            }
            FxAction::ClearSelection => {
                self.selected_keys.clear();
                self.selection_stats_dirty = true;
            }
            FxAction::DownloadSelected => {
                let mut keys: Vec<ItemKey> = self.selected_keys.iter().copied().collect();
                if keys.is_empty()
                    && let Some(fk) = self.focused_key
                {
                    keys.push(fk);
                }
                if !keys.is_empty() {
                    let opts = DownloadOpts {
                        out_dir: self.settings.out_dir.clone(),
                        format: self.settings.format,
                        batch_limit: self.settings.batch_limit,
                    };
                    let _ = core_tx.send(UiToWorkerCommand::EnqueueDownloads {
                        items: keys,
                        opts,
                        ack_soft_quota: false,
                        mode: bcvault_core::types::EnqueueMode::New,
                    });
                }
            }
            FxAction::DownloadRemaining(target) => {
                self.trigger_download_remaining(target, core_tx);
            }
            FxAction::DownloadKey(key) => {
                let opts = DownloadOpts {
                    out_dir: self.settings.out_dir.clone(),
                    format: self.settings.format,
                    batch_limit: self.settings.batch_limit,
                };
                let _ = core_tx.send(UiToWorkerCommand::EnqueueDownloads {
                    items: vec![key],
                    opts,
                    ack_soft_quota: false,
                    mode: bcvault_core::types::EnqueueMode::New,
                });
            }
            FxAction::Redownload(key) => {
                let opts = DownloadOpts {
                    out_dir: self.settings.out_dir.clone(),
                    format: self.settings.format,
                    batch_limit: self.settings.batch_limit,
                };
                let _ = core_tx.send(UiToWorkerCommand::EnqueueDownloads {
                    items: vec![key],
                    opts,
                    ack_soft_quota: false,
                    mode: bcvault_core::types::EnqueueMode::Force,
                });
            }
            FxAction::OpenInBrowser(key) => {
                if let Some(w) = self.rows.iter().find(|r| r.rel.key == key) {
                    let _ = open::that_detached(&w.rel.url);
                }
            }
            FxAction::OpenFolder(key) => {
                if let Some(w) = self.rows.iter().find(|r| r.rel.key == key)
                    && let Some(p) = &w.rel.local_path
                {
                    let target = if p.is_file() {
                        p.parent().unwrap_or(p)
                    } else {
                        p
                    };
                    let _ = open::that_detached(target);
                }
            }
            FxAction::RedeemDialog(key) => {
                if let Some(w) = self.rows.iter().find(|r| r.rel.key == key) {
                    self.redeem_modal = Some((key, w.rel.title.clone(), String::new()));
                }
            }
            FxAction::SetReleaseDownloaded(key, downloaded) => {
                let _ = core_tx.send(UiToWorkerCommand::SetReleaseDownloaded { key, downloaded });
            }
            FxAction::SetReleaseUnavailable(key, unavailable) => {
                let _ = core_tx.send(UiToWorkerCommand::SetReleaseUnavailable {
                    keys: vec![key],
                    unavailable,
                });
            }
            FxAction::RelinkReleaseModal(key) => {
                self.pending_relink_pick = Some(key);
            }
            FxAction::SyncSelected(mode) => {
                if let Some(bid) = self.query.base.band_id {
                    let _ = core_tx.send(UiToWorkerCommand::Sync {
                        scope: SyncScope::Selected(vec![bid]),
                        mode,
                    });
                }
            }
            FxAction::SyncAll(mode) => {
                let _ = core_tx.send(UiToWorkerCommand::Sync {
                    scope: SyncScope::All,
                    mode,
                });
            }
            FxAction::CardSize(delta) => {
                let new_idx = (self.prefs.card_size_idx as isize + delta as isize)
                    .clamp(0, (CARD_SIZES.len() - 1) as isize)
                    as usize;
                if new_idx != self.prefs.card_size_idx {
                    self.snapshot_for_flip(now_s);
                    self.prefs.card_size_idx = new_idx;
                    let _ = fx_tx.send(FxCommand::SetPref {
                        key: "card_size_idx".to_string(),
                        value: new_idx.to_string(),
                    });
                }
            }
            FxAction::ToggleSidebar => {
                self.sidebar_collapsed = !self.sidebar_collapsed;
            }
            FxAction::SetTheme(theme) => {
                self.prefs.theme = theme.clone();
                let _ = fx_tx.send(FxCommand::SetPref {
                    key: "theme".to_string(),
                    value: theme,
                });
            }
            FxAction::ReloadThemes => {
                if let Some(dir) = self.themes_dir.clone() {
                    let issues = self.themes.load_user_dir(&dir);
                    let _ = fx_tx.send(FxCommand::SetThemeCommands(self.themes.palette_commands()));
                    let user_count = self.themes.user_entries().count();
                    let msg = if issues.is_empty() {
                        format!("Reloaded {user_count} user theme(s)")
                    } else {
                        format!(
                            "Reloaded {user_count} user theme(s), {} problem(s)",
                            issues.len()
                        )
                    };
                    self.push_toast(
                        if issues.is_empty() {
                            ToastKind::Info
                        } else {
                            ToastKind::Warn
                        },
                        msg,
                        now_s,
                    );
                }
            }
            FxAction::OpenThemesFolder => {
                if let Some(dir) = &self.themes_dir {
                    let _ = open::that_detached(dir);
                }
            }
            FxAction::OpenThemeEditorNew => {
                if let Some(entry) = self.themes.resolve(&self.prefs.theme) {
                    let existing_ids: Vec<String> =
                        self.themes.entries().iter().map(|e| e.id.clone()).collect();
                    let editor = crate::overlays::theme_editor::ThemeEditorState::from_entry(
                        entry,
                        true,
                        &existing_ids,
                    );
                    self.theme_preview = Some(editor.file.to_fx_theme());
                    self.theme_editor = Some(editor);
                }
            }
            FxAction::OpenThemeEditorFor(id) => {
                if let Some(entry) = self.themes.get(&id) {
                    let is_builtin =
                        matches!(entry.source, crate::theme::registry::ThemeSource::Builtin);
                    let existing_ids: Vec<String> =
                        self.themes.entries().iter().map(|e| e.id.clone()).collect();
                    let editor = crate::overlays::theme_editor::ThemeEditorState::from_entry(
                        entry,
                        is_builtin,
                        &existing_ids,
                    );
                    self.theme_preview = Some(editor.file.to_fx_theme());
                    self.theme_editor = Some(editor);
                }
            }
            FxAction::ImportThemes => {
                self.pending_theme_import_pick = true;
            }
            FxAction::ExportTheme(id) => {
                if let Some(entry) = self.themes.get(&id) {
                    self.pending_theme_export_pick = Some(entry.file.clone());
                }
            }
            FxAction::SetAdaptiveAccent(enabled) => {
                self.prefs.adaptive_accent = enabled;
                let _ = fx_tx.send(FxCommand::SetPref {
                    key: "adaptive_accent".to_string(),
                    value: if enabled { "1" } else { "0" }.to_string(),
                });
            }
            FxAction::SetEffects(level) => {
                self.prefs.effects = level;
                self.motion.level = level;
                let _ = fx_tx.send(FxCommand::SetPref {
                    key: "effects".to_string(),
                    value: level.as_str().to_string(),
                });
            }
            FxAction::SetReducedMotion(rm) => {
                self.prefs.reduced_motion = rm;
                self.motion.reduced_motion = rm;
                let _ = fx_tx.send(FxCommand::SetPref {
                    key: "reduced_motion".to_string(),
                    value: if rm { "1" } else { "0" }.to_string(),
                });
            }
            FxAction::NavBack => {
                let curr = self.current_view_state();
                if let Some(prev) = self.nav.go_back(curr) {
                    self.view_mode = prev.view;
                    self.query = prev.query;
                    self.pending_scroll_to_y = Some(prev.scroll_y);
                    self.focused_key = prev.focus;
                    self.issue_wall_and_facets_query(fx_tx);
                }
            }
            FxAction::NavForward => {
                let curr = self.current_view_state();
                if let Some(next) = self.nav.go_forward(curr) {
                    self.view_mode = next.view;
                    self.query = next.query;
                    self.pending_scroll_to_y = Some(next.scroll_y);
                    self.focused_key = next.focus;
                    self.issue_wall_and_facets_query(fx_tx);
                }
            }
            FxAction::SaveCurrentView(name) => {
                let st = self.current_view_state();
                let _ = fx_tx.send(FxCommand::SaveView { name, state: st });
            }
            FxAction::ApplySavedView(id) => {
                if let Some(sv) = self.saved_views.iter().find(|v| v.id == id).cloned() {
                    self.nav.push(self.current_view_state());
                    self.view_mode = sv.state.view;
                    self.query = sv.state.query;
                    self.pending_scroll_to_y = Some(sv.state.scroll_y);
                    self.focused_key = sv.state.focus;
                    self.issue_wall_and_facets_query(fx_tx);
                }
            }
            FxAction::DeleteSavedView(id) => {
                let _ = fx_tx.send(FxCommand::DeleteView(id));
            }
            FxAction::PurgeCovers => {
                let _ = fx_tx.send(FxCommand::PurgeCovers);
            }
            FxAction::ToggleDebug => {
                self.show_debug_overlay = !self.show_debug_overlay;
            }
            FxAction::OpenLinkFolderModal(band_id) => {
                self.link_folder_modal = Some(FxLinkFolderModalState::new_empty(band_id));
            }
            FxAction::VerifyFiles => {
                let _ = core_tx.send(UiToWorkerCommand::VerifyFiles {
                    scope: SyncScope::All,
                });
            }
            FxAction::UndoBatch(batch_id) => {
                let _ = core_tx.send(UiToWorkerCommand::UndoBatch { batch_id });
            }
            FxAction::NewGroup => {
                self.group_error = None;
                self.group_editor = Some(crate::overlays::groups::FxGroupEditorState::new_manual(
                    Vec::new(),
                ));
            }
            FxAction::NewGroupFromSelection(ids) => {
                self.group_error = None;
                self.group_editor =
                    Some(crate::overlays::groups::FxGroupEditorState::new_manual(ids));
            }
            FxAction::EditGroup(id) => {
                if let Some(g) = self.groups.iter().find(|g| g.id == id) {
                    let members = self.group_member_ids(id);
                    self.group_error = None;
                    self.group_editor = Some(
                        crate::overlays::groups::FxGroupEditorState::from_group(g, &members),
                    );
                }
            }
            FxAction::DeleteGroup(id) => {
                if let Some(g) = self.groups.iter().find(|g| g.id == id) {
                    let pin_count = self.pins.values().filter(|&&gid| gid == id).count();
                    self.confirm_delete_group = Some((id, g.name.clone(), pin_count));
                }
            }
            FxAction::SetActiveGrouping(grouping) => {
                let _ = core_tx.send(UiToWorkerCommand::SetActiveGrouping { grouping });
            }
            FxAction::NewGroupSet => {
                self.group_error = None;
                self.group_set_modal = Some(crate::overlays::groups::FxGroupSetModalState::new());
            }
            FxAction::RenameGroupSet(id) => {
                if let Some(set) = self.group_sets.iter().find(|s| s.id == id) {
                    self.group_error = None;
                    self.group_set_modal =
                        Some(crate::overlays::groups::FxGroupSetModalState::rename(set));
                }
            }
            FxAction::DeleteGroupSet(id) => {
                if let Some(set) = self.group_sets.iter().find(|s| s.id == id) {
                    self.confirm_delete_group_set = Some((id, set.name.clone()));
                }
            }
            FxAction::ToggleGroup(id) => {
                if id < 0 {
                    if self.collapsed_builtin_groups.contains(&id) {
                        self.collapsed_builtin_groups.remove(&id);
                    } else {
                        self.collapsed_builtin_groups.insert(id);
                    }
                    self.sidebar_dirty = true;
                } else if let Some(g) = self.groups.iter().find(|g| g.id == id) {
                    let _ = core_tx.send(UiToWorkerCommand::SetArtistGroupCollapsed {
                        id,
                        collapsed: !g.collapsed,
                    });
                }
            }
            FxAction::ToggleUngrouped => {
                self.ungrouped_collapsed = !self.ungrouped_collapsed;
                self.sidebar_dirty = true;
            }
            FxAction::CollapseAllGroups => {
                match self.active_grouping {
                    bcvault_core::groups::ActiveGrouping::CustomSet(_) => {
                        for g in &self.groups {
                            if !g.collapsed {
                                let _ = core_tx.send(UiToWorkerCommand::SetArtistGroupCollapsed {
                                    id: g.id,
                                    collapsed: true,
                                });
                            }
                        }
                        self.ungrouped_collapsed = true;
                    }
                    bcvault_core::groups::ActiveGrouping::Builtin(_) => {
                        for g in &self.builtin_groups {
                            self.collapsed_builtin_groups.insert(g.id);
                        }
                    }
                    bcvault_core::groups::ActiveGrouping::None => {}
                }
                self.sidebar_dirty = true;
            }
            FxAction::ExpandAllGroups => {
                match self.active_grouping {
                    bcvault_core::groups::ActiveGrouping::CustomSet(_) => {
                        for g in &self.groups {
                            if g.collapsed {
                                let _ = core_tx.send(UiToWorkerCommand::SetArtistGroupCollapsed {
                                    id: g.id,
                                    collapsed: false,
                                });
                            }
                        }
                        self.ungrouped_collapsed = false;
                    }
                    bcvault_core::groups::ActiveGrouping::Builtin(_) => {
                        self.collapsed_builtin_groups.clear();
                    }
                    bcvault_core::groups::ActiveGrouping::None => {}
                }
                self.sidebar_dirty = true;
            }
            FxAction::MoveArtistsToGroup(band_ids, target) => {
                if let bcvault_core::groups::ActiveGrouping::CustomSet(set_id) =
                    self.active_grouping
                {
                    let _ = core_tx.send(UiToWorkerCommand::PinArtists {
                        set_id,
                        band_ids,
                        target,
                    });
                }
            }
            FxAction::SyncGroup(id) => {
                let members = self.group_member_ids(id);
                if !members.is_empty() {
                    let _ = core_tx.send(UiToWorkerCommand::Sync {
                        scope: SyncScope::Selected(members),
                        mode: self.sync_mode,
                    });
                }
            }
            FxAction::DownloadRemainingGroup(id) => {
                let members = self.group_member_ids(id);
                let title = self
                    .groups
                    .iter()
                    .chain(self.builtin_groups.iter())
                    .find(|g| g.id == id)
                    .map(|g| format!("Download Remaining for {}", g.name))
                    .unwrap_or_else(|| "Download Remaining for Group".to_string());
                self.trigger_download_remaining_artists(title, members, core_tx);
            }
            FxAction::ViewGroup(id) => {
                let members = self.group_member_ids(id);
                self.nav.push(self.current_view_state());
                if members.is_empty() {
                    self.query.base.band_id = Some(-1);
                    self.query.base.band_ids.clear();
                } else if members.len() == 1 {
                    self.query.base.band_id = Some(members[0]);
                    self.query.base.band_ids.clear();
                } else {
                    self.query.base.band_id = None;
                    self.query.base.band_ids = members;
                }
                self.issue_wall_and_facets_query(fx_tx);
            }
        }
    }
}

/// Paints a cover image at `rect` with `tiny8` 8x8 blocky blur-up or geometric placeholder,
/// cross-fading smoothly once the texture is uploaded, and optional UV parallax/zoom.
#[allow(clippy::too_many_arguments)]
pub fn paint_cover_slot(
    painter: &egui::Painter,
    state: &mut FxState,
    art_id: Option<i64>,
    seed_id: i64,
    rect: Rect,
    tier: Tier,
    zoom: f32,
    uv_shift: Vec2,
) {
    let th = state.active_theme;
    let tex_ready = art_id.is_some_and(|id| state.tex_cache.contains((id, tier)));
    let alpha_target = if tex_ready { 1.0 } else { 0.0 };
    let img_alpha = state.motion.val(
        ("cov_fade", art_id.unwrap_or(seed_id), tier),
        alpha_target,
        SpringCfg::SMOOTH,
    );

    if img_alpha < 0.999 {
        if let Some(id) = art_id
            && let Some(pal) = state.palettes.get(&id)
        {
            crate::paint::tiny8_mosaic(painter, rect, &pal.tiny8, 1.0);
        } else {
            crate::paint::geometric_placeholder(
                painter,
                rect,
                art_id.unwrap_or(seed_id),
                th.bg2,
                th.accent,
                th.accent2,
            );
        }
    }

    if let Some(id) = art_id {
        let tex_id_opt = state
            .tex_cache
            .get((id, tier))
            .map(|h| h.id())
            .or_else(|| state.tex_cache.get((id, Tier::T192)).map(|h| h.id()))
            .or_else(|| state.tex_cache.get((id, Tier::T64)).map(|h| h.id()));

        if let Some(tex_id) = tex_id_opt {
            let z = zoom.max(1.0);
            let span = 1.0 / z;
            let base = (1.0 - span) * 0.5;
            let u0 = (base + uv_shift.x).clamp(0.0, 1.0 - span);
            let v0 = (base + uv_shift.y).clamp(0.0, 1.0 - span);
            let uv = Rect::from_min_max(egui::pos2(u0, v0), egui::pos2(u0 + span, v0 + span));
            let tint = egui::Color32::WHITE.gamma_multiply(img_alpha);
            painter.image(tex_id, rect, uv, tint);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bcvault_core::types::{Classification, ItemType, ReleaseFilter, ReleaseRow};
    use tokio::sync::mpsc;

    #[test]
    fn discards_stale_wall_query_chunks_and_dispatches_actions() {
        let mut state = FxState::new();
        let (core_tx, _core_rx) = mpsc::unbounded_channel();
        let (fx_tx, _fx_rx) = mpsc::unbounded_channel();

        state.dispatch(
            FxAction::SetFilter(ReleaseFilter::Free),
            &core_tx,
            &fx_tx,
            0.0,
        );
        let newest_req = state.active_wall_req_id;

        let dummy_row = WallRow {
            rel: ReleaseRow {
                key: ItemKey {
                    kind: ItemType::Album,
                    id: 1,
                },
                band_id: 1,
                title: "A".to_string(),
                artist_name: "B".to_string(),
                url: "https://b.bandcamp.com/album/a".to_string(),
                cover_url: None,
                art_id: Some(10),
                classification: Classification::Free,
                is_preorder: false,
                track_count: Some(4),
                published_at: None,
                modified_at: None,
                downloaded: false,
                downloaded_at: None,
                downloaded_format: None,
                local_path: None,
                local_status: None,
                origin: None,
                missing_ack_at: None,
                has_remote_update: false,
                remote_update_reason: None,
            },
            art_id: Some(10),
            hue: Some(180),
            accent: Some([0, 200, 180]),
            added_at: 100,
        };

        // Stale request id is ignored
        state.apply_fx_event(
            FxEvent::WallLoaded {
                request_id: newest_req.saturating_sub(1),
                chunk: 0,
                last: true,
                rows: vec![dummy_row.clone()],
            },
            0.0,
        );
        assert!(state.rows.is_empty());

        // Matching request id is applied
        state.apply_fx_event(
            FxEvent::WallLoaded {
                request_id: newest_req,
                chunk: 0,
                last: true,
                rows: vec![dummy_row],
            },
            0.0,
        );
        assert_eq!(state.rows.len(), 1);
    }

    #[test]
    fn test_link_folder_modal_state() {
        let modal = FxLinkFolderModalState::new_empty(Some(42));
        assert_eq!(modal.band_id, Some(42));
        assert!(modal.folder_path.is_empty());
        assert!(modal.auto_apply_confident);
        assert!(modal.error.is_none());

        if let Ok(mut lock) = modal.picked_folder.lock() {
            *lock = Some(std::path::PathBuf::from("/music/artist42"));
        }
        let picked = modal.picked_folder.lock().ok().and_then(|mut l| l.take());
        assert_eq!(picked, Some(std::path::PathBuf::from("/music/artist42")));

        let repoint = FxLinkFolderModalState::for_repoint(42, 7, "/music/artist42_old".to_string());
        assert_eq!(repoint.band_id, Some(42));
        assert_eq!(repoint.repoint_folder_id, Some(7));
        assert_eq!(repoint.folder_path, "/music/artist42_old");
    }

    #[test]
    fn test_worst_folder_state_and_display_label() {
        use bcvault_core::types::FolderState;
        use std::path::PathBuf;

        assert_eq!(worst_folder_state(&[]), None);

        let f_ok = ArtistFolder {
            id: 1,
            band_id: 10,
            path: PathBuf::from("/ambient/Music"),
            state: FolderState::Ok,
        };
        let f_unreadable = ArtistFolder {
            id: 2,
            band_id: 10,
            path: PathBuf::from("/techno/Music"),
            state: FolderState::Unreadable,
        };
        let f_missing = ArtistFolder {
            id: 3,
            band_id: 10,
            path: PathBuf::from("/other/Archive"),
            state: FolderState::Missing,
        };

        assert_eq!(
            worst_folder_state(std::slice::from_ref(&f_ok)),
            Some(FolderState::Ok)
        );
        assert_eq!(
            worst_folder_state(&[f_ok.clone(), f_unreadable.clone()]),
            Some(FolderState::Unreadable)
        );
        assert_eq!(
            worst_folder_state(&[f_ok.clone(), f_missing.clone()]),
            Some(FolderState::Missing)
        );
        assert_eq!(
            worst_folder_state(&[f_missing.clone(), f_unreadable.clone()]),
            Some(FolderState::Missing)
        );

        // folder_display_label
        assert_eq!(
            folder_display_label(&f_missing, &[f_ok.clone(), f_missing.clone()]),
            "Archive"
        );

        let all = vec![f_ok.clone(), f_unreadable.clone()];
        assert_eq!(folder_display_label(&f_ok, &all), "ambient/Music");
        assert_eq!(folder_display_label(&f_unreadable, &all), "techno/Music");
    }

    #[test]
    fn test_reconcile_report_modal_state() {
        use bcvault_core::library::matcher::{MatchProposal, Score, ScoredCandidate};
        let key1 = ItemKey {
            kind: ItemType::Album,
            id: 1,
        };
        let key2 = ItemKey {
            kind: ItemType::Album,
            id: 2,
        };
        let report = Box::new(ReconcileReport {
            batch_id: 100,
            band_id: 42,
            matched: vec![MatchProposal {
                release_key: key1,
                release_title: "Album 1".to_string(),
                candidate_path: std::path::PathBuf::from("/music/artist42/Album 1"),
                candidate_name: "Album 1".to_string(),
                score: Score {
                    value: 0.98,
                    signals: vec![],
                },
                format_guess: None,
            }],
            review: vec![bcvault_core::library::matcher::ReviewItem {
                release_key: key2,
                release_title: "Album 2".to_string(),
                candidates: vec![ScoredCandidate {
                    candidate_path: std::path::PathBuf::from("/music/artist42/Album 2 (Deluxe)"),
                    candidate_name: "Album 2 (Deluxe)".to_string(),
                    score: Score {
                        value: 0.82,
                        signals: vec![],
                    },
                    format_guess: None,
                }],
            }],
            page_only: vec![],
            local_only: vec![],
            conflicts: vec![],
            restorable: vec![],
            linked: vec![],
            unavailable_folders: vec![],
            catalog: vec![],
        });

        let mut modal = FxReconcileReportModalState::new(report);
        assert_eq!(modal.active_tab, FxReportTab::Matched);
        assert_eq!(modal.total_selected_count(), 2);

        modal.selected_matched.clear();
        assert_eq!(modal.total_selected_count(), 1);

        modal.selected_review.clear();
        assert_eq!(modal.total_selected_count(), 0);
    }

    #[test]
    fn test_resolve_missing_modal_state() {
        let key1 = ItemKey {
            kind: ItemType::Album,
            id: 1,
        };
        let key2 = ItemKey {
            kind: ItemType::Album,
            id: 2,
        };
        let items = vec![
            MissingItem {
                key: key1,
                band_id: 42,
                title: "Album 1".to_string(),
                artist_name: "Artist 42".to_string(),
                last_known_path: Some(std::path::PathBuf::from("/music/artist42/Album 1")),
                best_candidate: Some(bcvault_core::types::MissingCandidate {
                    path: std::path::PathBuf::from("/music/artist42/Album 1 (FLAC)"),
                    score: 0.95,
                }),
            },
            MissingItem {
                key: key2,
                band_id: 42,
                title: "Album 2".to_string(),
                artist_name: "Artist 42".to_string(),
                last_known_path: Some(std::path::PathBuf::from("/music/artist42/Album 2")),
                best_candidate: None,
            },
        ];

        let modal = FxResolveMissingModalState::new(200, items, false, DeletedPolicy::KeepMarked);

        // Key1 has a candidate -> default is Moved
        assert!(matches!(
            modal.decisions.get(&key1),
            Some(FxMissingItemChoice::Moved(_))
        ));
        // Key2 has no candidate -> default is Deleted
        assert!(matches!(
            modal.decisions.get(&key2),
            Some(FxMissingItemChoice::Deleted)
        ));
    }

    #[test]
    fn test_library_events_and_dispatches() {
        let mut state = FxState::new();
        let (core_tx, mut core_rx) = mpsc::unbounded_channel();
        let (fx_tx, _fx_rx) = mpsc::unbounded_channel();

        // 1. Ready event sets folders
        let f1 = ArtistFolder {
            id: 1,
            band_id: 10,
            path: std::path::PathBuf::from("/music/artist10"),
            state: bcvault_core::types::FolderState::Ok,
        };
        state.apply_core_event(
            WorkerToUiEvent::Ready {
                artists: vec![],
                settings: Settings::default_with_dir(std::path::PathBuf::from("/out")),
                folders: vec![f1],
                active_grouping: bcvault_core::groups::ActiveGrouping::None,
                group_sets: vec![],
                groups: vec![],
                pins: std::collections::HashMap::new(),
            },
            &fx_tx,
            0.0,
        );
        assert_eq!(state.folders.len(), 1);

        // 2. FolderStatus updates folder
        let f1_missing = ArtistFolder {
            id: 1,
            band_id: 10,
            path: std::path::PathBuf::from("/music/artist10"),
            state: bcvault_core::types::FolderState::Missing,
        };
        state.apply_core_event(
            WorkerToUiEvent::FolderStatus {
                band_id: 10,
                folders: vec![f1_missing],
            },
            &fx_tx,
            0.0,
        );
        assert_eq!(
            state
                .folders
                .get(&10)
                .and_then(|v| v.first())
                .map(|f| f.state),
            Some(bcvault_core::types::FolderState::Missing)
        );

        // 3. FolderStatus with empty folders removes folder
        state.apply_core_event(
            WorkerToUiEvent::FolderStatus {
                band_id: 10,
                folders: vec![],
            },
            &fx_tx,
            0.0,
        );
        assert!(state.folders.is_empty());

        // 4. LibraryApplied pushes a toast with undo_batch_id
        state.apply_core_event(
            WorkerToUiEvent::LibraryApplied {
                batch_id: 55,
                summary: bcvault_core::types::BatchSummary {
                    adopted: 3,
                    relinked: 1,
                    archived: 0,
                    restored: 0,
                    ignored: 0,
                    unmarked: 0,
                },
            },
            &fx_tx,
            0.0,
        );
        assert_eq!(state.toasts.len(), 1);
        assert_eq!(state.toasts[0].undo_batch_id, Some(55));

        // 5. Dispatch actions
        state.dispatch(
            FxAction::OpenLinkFolderModal(Some(10)),
            &core_tx,
            &fx_tx,
            0.0,
        );
        assert!(state.link_folder_modal.is_some());

        state.dispatch(FxAction::VerifyFiles, &core_tx, &fx_tx, 0.0);
        let cmd = core_rx.try_recv().ok();
        assert!(matches!(cmd, Some(UiToWorkerCommand::VerifyFiles { .. })));

        state.dispatch(FxAction::UndoBatch(55), &core_tx, &fx_tx, 0.0);
        let cmd = core_rx.try_recv().ok();
        assert!(matches!(
            cmd,
            Some(UiToWorkerCommand::UndoBatch { batch_id: 55 })
        ));
    }

    #[test]
    fn test_escape_key_closes_top_modal_in_priority_order() {
        let mut state = FxState::new();
        assert!(!state.has_open_modal());
        assert!(!state.close_top_modal());

        state.settings_modal_open = true;
        assert!(state.has_open_modal());

        state.link_folder_modal = Some(FxLinkFolderModalState::new_empty(Some(1)));
        assert!(state.has_open_modal());

        state.reconcile_report_modal = Some(FxReconcileReportModalState::new(Box::new(
            bcvault_core::library::service::ReconcileReport {
                batch_id: 1,
                band_id: 1,
                matched: vec![],
                review: vec![],
                page_only: vec![],
                local_only: vec![],
                conflicts: vec![],
                restorable: vec![],
                linked: vec![],
                unavailable_folders: vec![],
                catalog: vec![],
            },
        )));
        assert!(state.has_open_modal());

        // Closes reconcile report modal first
        assert!(state.close_top_modal());
        assert!(state.reconcile_report_modal.is_none());
        assert!(state.has_open_modal());

        // Closes link folder modal second
        assert!(state.close_top_modal());
        assert!(state.link_folder_modal.is_none());
        assert!(state.has_open_modal());

        // Closes settings modal third
        assert!(state.close_top_modal());
        assert!(!state.settings_modal_open);
        assert!(!state.has_open_modal());

        assert!(!state.close_top_modal());
    }

    #[test]
    fn test_fx_unavailable_modal_state_and_chip() {
        use bcvault_core::types::{
            DeletedPolicy, FolderState, ItemType, UnavailableFolder, UnavailableRelease,
            UnavailableRoot,
        };
        use std::path::PathBuf;

        let mut st = FxState::new();
        let (fx_tx, _fx_rx) = tokio::sync::mpsc::unbounded_channel();

        let folder = UnavailableFolder {
            root: UnavailableRoot::Linked { folder_id: 42 },
            path: PathBuf::from("/music/aphex"),
            state: FolderState::Missing,
            releases: vec![UnavailableRelease {
                key: ItemKey::new(ItemType::Album, 100),
                band_id: 42,
                artist_name: "Aphex Twin".to_string(),
                title: "SAW II".to_string(),
                last_known_path: Some(PathBuf::from("/music/aphex/saw2")),
            }],
        };

        // 1. prompt: false does NOT open the modal
        st.apply_core_event(
            WorkerToUiEvent::FolderUnavailable {
                batch_id: 50,
                folders: vec![folder.clone()],
                prompt: false,
            },
            &fx_tx,
            0.0,
        );
        assert!(!st.show_unavailable_window);
        assert!(st.unavailable_modal.is_none());

        // 2. prompt: true opens the modal
        st.apply_core_event(
            WorkerToUiEvent::FolderUnavailable {
                batch_id: 51,
                folders: vec![folder],
                prompt: true,
            },
            &fx_tx,
            0.0,
        );
        assert!(st.show_unavailable_window);
        assert!(st.unavailable_modal.is_some());
        if let Some(modal) = &st.unavailable_modal {
            assert_eq!(modal.batch_id, 51);
            assert_eq!(modal.cards.len(), 1);
            assert!(modal.cards[0].unlink, "Linked roots default unlink to true");
            assert_eq!(modal.cards[0].policy, DeletedPolicy::KeepMarked);
        } else {
            unreachable!("modal must be Some");
        }

        // 3. unavailable_folder_artists tracks state.folders with state != Ok
        assert_eq!(st.unavailable_folder_artists().len(), 0);
        st.folders.insert(
            42,
            vec![bcvault_core::types::ArtistFolder {
                id: 1,
                band_id: 42,
                path: PathBuf::from("/music/aphex"),
                state: FolderState::Missing,
            }],
        );
        st.folders.insert(
            43,
            vec![bcvault_core::types::ArtistFolder {
                id: 2,
                band_id: 43,
                path: PathBuf::from("/music/boc"),
                state: FolderState::Ok,
            }],
        );
        assert_eq!(st.unavailable_folder_artists(), vec![42]);
    }

    #[test]
    fn test_22_fx_state_multi_folder_lifecycle() {
        let mut st = FxState::default();
        let (fx_tx, _fx_rx) = tokio::sync::mpsc::unbounded_channel();
        let f1_a1 = ArtistFolder {
            id: 1,
            band_id: 10,
            path: std::path::PathBuf::from("/music/artist10_a"),
            state: FolderState::Ok,
        };
        let f2_a1 = ArtistFolder {
            id: 2,
            band_id: 10,
            path: std::path::PathBuf::from("/music/artist10_b"),
            state: FolderState::Missing,
        };
        let f3_a2 = ArtistFolder {
            id: 3,
            band_id: 20,
            path: std::path::PathBuf::from("/music/artist20"),
            state: FolderState::Ok,
        };

        st.apply_core_event(
            WorkerToUiEvent::Ready {
                artists: vec![],
                settings: Settings::default(),
                folders: vec![f1_a1.clone(), f2_a1.clone(), f3_a2.clone()],
                active_grouping: bcvault_core::groups::ActiveGrouping::None,
                group_sets: vec![],
                groups: vec![],
                pins: std::collections::HashMap::new(),
            },
            &fx_tx,
            0.0,
        );

        assert_eq!(st.folders.get(&10).map(|v| v.len()), Some(2));
        assert_eq!(st.folders.get(&20).map(|v| v.len()), Some(1));

        st.apply_core_event(
            WorkerToUiEvent::FolderStatus {
                band_id: 10,
                folders: vec![f1_a1],
            },
            &fx_tx,
            0.0,
        );
        assert_eq!(st.folders.get(&10).map(|v| v.len()), Some(1));
        assert_eq!(st.folders.get(&20).map(|v| v.len()), Some(1));

        st.apply_core_event(
            WorkerToUiEvent::FolderStatus {
                band_id: 10,
                folders: vec![],
            },
            &fx_tx,
            0.0,
        );
        assert!(!st.folders.contains_key(&10));
        assert_eq!(st.folders.get(&20).map(|v| v.len()), Some(1));
    }

    #[test]
    fn test_fx_reconcile_modal_mark_wrong_and_conflict_resolution() {
        use bcvault_core::library::matcher::{Conflict, ReviewItem, Score, ScoredCandidate};
        use bcvault_core::types::MatchDecision;
        use std::path::PathBuf;

        let k1 = ItemKey {
            kind: ItemType::Album,
            id: 10,
        };
        let k2 = ItemKey {
            kind: ItemType::Album,
            id: 20,
        };
        let cand_path = PathBuf::from("/music/Wall Split Action 001");
        let conflict_path = PathBuf::from("/music/split");

        let report = Box::new(ReconcileReport {
            batch_id: 99,
            band_id: 7,
            matched: vec![],
            review: vec![ReviewItem {
                release_key: k1,
                release_title: "split".to_string(),
                candidates: vec![ScoredCandidate {
                    candidate_path: cand_path.clone(),
                    candidate_name: "Wall Split Action 001".to_string(),
                    score: Score {
                        value: 0.85,
                        signals: vec![],
                    },
                    format_guess: None,
                }],
            }],
            page_only: vec![],
            local_only: vec![],
            conflicts: vec![Conflict {
                description: "Folder 'split' matches 2 releases".to_string(),
                release_keys: vec![k1, k2],
                candidate_paths: vec![conflict_path.clone()],
            }],
            restorable: vec![],
            linked: vec![],
            unavailable_folders: vec![],
            catalog: vec![],
        });

        let mut modal = FxReconcileReportModalState::new(report);
        modal.mark_all_review_wrong();
        assert!(modal.selected_review.is_empty());
        assert_eq!(modal.rejected_review.len(), 1);

        modal
            .conflict_decisions
            .insert(conflict_path.clone(), FxLocalAction::Attach(k2));
        assert_eq!(modal.total_selected_count(), 2);

        let decisions = modal.build_decisions();
        assert_eq!(decisions.len(), 2);
        assert!(decisions.contains(&MatchDecision::Reject {
            band_id: 7,
            key: k1,
            path: cand_path,
        }));
        assert!(decisions.contains(&MatchDecision::Attach {
            key: k2,
            path: conflict_path,
            format: None,
            folder_mtime: None,
        }));
    }

    #[test]
    fn theme_preview_overrides_resolved_theme_and_disables_adaptive_accent() {
        let mut state = FxState::new();
        let base = state.resolved_theme();
        assert_eq!(base, state.themes.resolve_theme("carbon"));

        let swatch = Some([0xFF, 0x00, 0x80]);
        let adapted = state.resolved_target_theme(swatch);
        assert_ne!(adapted.accent, base.accent);

        let preview = state.themes.resolve_theme("paper");
        state.theme_preview = Some(preview);
        assert_eq!(state.resolved_theme(), preview);
        assert_eq!(state.resolved_target_theme(swatch), preview);

        state.theme_preview = None;
        assert_eq!(state.resolved_theme(), base);
    }

    #[test]
    fn test_14_fx_quota_warnings_dedup_continue_and_cancel() {
        use bcvault_core::types::{AudioFormat, EnqueueMode, ItemType};

        let mut state = FxState::new();
        let (core_tx, mut core_rx) = tokio::sync::mpsc::unbounded_channel();
        let (fx_tx, _fx_rx) = tokio::sync::mpsc::unbounded_channel();
        let opts = DownloadOpts {
            format: AudioFormat::Flac,
            out_dir: std::path::PathBuf::from("/tmp/out"),
            batch_limit: 50,
        };

        state.apply_core_event(
            WorkerToUiEvent::QuotaWarning {
                band_id: 1,
                used_30d: 145,
                requested: 10,
                soft: 150,
                hard: 200,
                items: vec![ItemKey::new(ItemType::Album, 11)],
                opts: opts.clone(),
            },
            &fx_tx,
            0.0,
        );
        state.apply_core_event(
            WorkerToUiEvent::QuotaWarning {
                band_id: 2,
                used_30d: 148,
                requested: 5,
                soft: 150,
                hard: 200,
                items: vec![ItemKey::new(ItemType::Album, 22)],
                opts: opts.clone(),
            },
            &fx_tx,
            0.0,
        );
        assert_eq!(state.quota_warnings.len(), 2);
        assert!(state.has_open_modal());

        state.confirm_quota_warnings(&core_tx);
        assert!(state.quota_warnings.is_empty());
        assert!(!state.has_open_modal());
        match core_rx.try_recv() {
            Ok(UiToWorkerCommand::EnqueueDownloads {
                items,
                ack_soft_quota,
                mode,
                ..
            }) => {
                assert!(ack_soft_quota);
                assert_eq!(mode, EnqueueMode::New);
                assert_eq!(
                    items,
                    vec![
                        ItemKey::new(ItemType::Album, 11),
                        ItemKey::new(ItemType::Album, 22)
                    ]
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }

        // Cancel via close_top_modal clears warnings without sending
        state.apply_core_event(
            WorkerToUiEvent::QuotaWarning {
                band_id: 3,
                used_30d: 149,
                requested: 2,
                soft: 150,
                hard: 200,
                items: vec![ItemKey::new(ItemType::Album, 33)],
                opts,
            },
            &fx_tx,
            0.0,
        );
        assert!(state.close_top_modal());
        assert!(state.quota_warnings.is_empty());
        assert!(core_rx.try_recv().is_err());
    }

    #[test]
    fn test_13_fx_selection_actions_and_stats_cache_invalidation() {
        let mut state = FxState::new();
        let (core_tx, _core_rx) = tokio::sync::mpsc::unbounded_channel();
        let (fx_tx, _fx_rx) = tokio::sync::mpsc::unbounded_channel();

        let make_row = |id: i64, downloaded: bool, unavailable: bool| WallRow {
            rel: ReleaseRow {
                key: ItemKey {
                    kind: ItemType::Album,
                    id,
                },
                band_id: 10,
                title: format!("Album {id}"),
                artist_name: "Artist".into(),
                url: format!("https://a.bandcamp.com/album/{id}"),
                cover_url: None,
                art_id: None,
                classification: if unavailable {
                    Classification::Unavailable
                } else {
                    Classification::Free
                },
                is_preorder: false,
                track_count: Some(5),
                published_at: None,
                modified_at: None,
                downloaded,
                downloaded_at: None,
                downloaded_format: None,
                local_path: None,
                local_status: None,
                origin: None,
                missing_ack_at: None,
                has_remote_update: false,
                remote_update_reason: None,
            },
            art_id: None,
            hue: None,
            accent: None,
            added_at: 0,
        };

        state.rows = vec![
            make_row(1, false, false),
            make_row(2, true, false),
            make_row(3, false, true),
            make_row(4, false, false),
        ];
        state.mark_selection_dirty();

        // Initial stats computation
        let s0 = state.selection_stats();
        assert_eq!(s0.total_in_view, 4);
        assert_eq!(s0.remaining_in_view, 2);
        assert_eq!(s0.selected, 0);
        let calls_after_first = state.selection_recount_calls;
        assert_eq!(calls_after_first, 1);

        // Repeated call without changes must use cache
        let _ = state.selection_stats();
        assert_eq!(state.selection_recount_calls, calls_after_first);

        // SelectRemaining selects only items 1 and 4
        state.dispatch(FxAction::SelectRemaining, &core_tx, &fx_tx, 0.0);
        assert_eq!(state.selected_keys.len(), 2);
        assert!(
            state
                .selected_keys
                .contains(&ItemKey::new(ItemType::Album, 1))
        );
        assert!(
            state
                .selected_keys
                .contains(&ItemKey::new(ItemType::Album, 4))
        );

        // InvertSelection selects items 2 and 3
        state.dispatch(FxAction::InvertSelection, &core_tx, &fx_tx, 0.0);
        assert_eq!(state.selected_keys.len(), 2);
        assert!(
            state
                .selected_keys
                .contains(&ItemKey::new(ItemType::Album, 2))
        );
        assert!(
            state
                .selected_keys
                .contains(&ItemKey::new(ItemType::Album, 3))
        );

        // SelectAll selects all 4
        state.dispatch(FxAction::SelectAll, &core_tx, &fx_tx, 0.0);
        assert_eq!(state.selected_keys.len(), 4);
        let s_all = state.selection_stats();
        assert_eq!(s_all.selected, 4);
        assert_eq!(s_all.selected_downloadable, 3);

        // ReleaseUpdated invalidates cache and updates remaining count
        let mut updated = state.rows[0].rel.clone();
        updated.downloaded = true;
        state.apply_core_event(
            WorkerToUiEvent::ReleaseUpdated(Box::new(updated)),
            &fx_tx,
            0.0,
        );
        let calls_before = state.selection_recount_calls;
        let s_after = state.selection_stats();
        assert_eq!(state.selection_recount_calls, calls_before + 1);
        assert_eq!(s_after.remaining_in_view, 1);
        assert_eq!(s_after.selected_downloadable, 3);
    }

    #[test]
    fn test_16_fx_bulk_confirmation_thresholds() {
        let mut state = FxState::new();
        let (core_tx, mut core_rx) = tokio::sync::mpsc::unbounded_channel();

        let make_artist =
            |band_id: i64, eligible: u32, eligible_downloaded: u32, used: u32| ArtistView {
                band_id,
                slug: format!("artist-{band_id}"),
                url: format!("https://artist-{band_id}.bandcamp.com"),
                name: format!("Artist {band_id}"),
                custom_name: None,
                last_synced_at: None,
                stats: bcvault_core::types::ArtistStats {
                    downloaded: eligible_downloaded,
                    eligible_downloaded,
                    non_eligible_downloaded: 0,
                    eligible,
                    total: eligible,
                    email_required: 0,
                    unknown: 0,
                    quota_used_30d: used,
                    archived: 0,
                    missing_files: 0,
                },
            };

        // 1) Small single-artist batch (< 25, no deferrals) -> direct EnqueueRemaining, no modal
        state.artists = vec![make_artist(1, 5, 0, 0)];
        state.query.base.band_id = Some(1);
        state.trigger_download_remaining(RemainingTarget::ThisArtist, &core_tx);
        assert!(state.bulk_confirm_modal.is_none());
        match core_rx.try_recv() {
            Ok(UiToWorkerCommand::EnqueueRemaining { scope, .. }) => {
                assert_eq!(scope, bcvault_core::types::RemainingScope::Artists(vec![1]));
            }
            other => panic!("expected EnqueueRemaining, got {other:?}"),
        }

        // 2) Single artist >= 25 remaining -> opens bulk confirmation modal
        state.artists = vec![make_artist(1, 30, 0, 0)];
        state.trigger_download_remaining(RemainingTarget::ThisArtist, &core_tx);
        assert!(state.bulk_confirm_modal.is_some());
        assert!(state.has_open_modal());
        assert!(core_rx.try_recv().is_err());

        // Cancel via close_top_modal does not send
        assert!(state.close_top_modal());
        assert!(state.bulk_confirm_modal.is_none());
        assert!(core_rx.try_recv().is_err());

        // 3) AllArtists always opens bulk confirmation modal and confirm_bulk_modal sends
        state.artists = vec![make_artist(1, 2, 0, 0), make_artist(2, 3, 0, 0)];
        state.trigger_download_remaining(RemainingTarget::AllArtists, &core_tx);
        let Some(modal) = state.bulk_confirm_modal.as_ref() else {
            panic!("AllArtists should open bulk confirm modal");
        };
        assert_eq!(modal.total_new, 5);
        assert_eq!(modal.total_queue, 5);
        assert_eq!(modal.total_deferred, 0);

        state.confirm_bulk_modal(&core_tx);
        assert!(state.bulk_confirm_modal.is_none());
        match core_rx.try_recv() {
            Ok(UiToWorkerCommand::EnqueueRemaining {
                scope,
                ack_soft_quota,
                ..
            }) => {
                assert_eq!(scope, bcvault_core::types::RemainingScope::Library);
                assert!(ack_soft_quota);
            }
            other => panic!("expected EnqueueRemaining, got {other:?}"),
        }
    }

    #[test]
    fn test_14_fx_sidebar_groups_dirty_cache_and_actions() {
        use bcvault_core::groups::{ActiveGrouping, ArtistGroup, ArtistGroupSet};
        use bcvault_core::types::ArtistStats;

        let mut state = FxState::new();
        let (core_tx, mut core_rx) = tokio::sync::mpsc::unbounded_channel();
        let (fx_tx, _fx_rx) = tokio::sync::mpsc::unbounded_channel();

        let a1 = ArtistView {
            band_id: 1,
            slug: "alpha".into(),
            name: "Alpha".into(),
            custom_name: None,
            url: "https://alpha.bandcamp.com".into(),
            last_synced_at: None,
            stats: ArtistStats::default(),
        };
        let s1 = ArtistGroupSet {
            id: 1,
            name: "Workflow".into(),
            pos: 0,
        };
        let g1 = ArtistGroup {
            id: 10,
            set_id: 1,
            name: "Priority".into(),
            pos: 0,
            collapsed: false,
        };

        state.apply_core_event(
            WorkerToUiEvent::Ready {
                artists: vec![a1],
                settings: Settings::default(),
                folders: vec![],
                active_grouping: ActiveGrouping::CustomSet(1),
                group_sets: vec![s1],
                groups: vec![g1],
                pins: HashMap::from([(1, 10)]),
            },
            &fx_tx,
            0.0,
        );

        state.ensure_sidebar_rows(1000);
        assert_eq!(state.sidebar_recompute_count, 1);
        state.ensure_sidebar_rows(1010);
        assert_eq!(state.sidebar_recompute_count, 1);

        state.dispatch(FxAction::ToggleGroup(10), &core_tx, &fx_tx, 0.0);
        assert!(matches!(
            core_rx.try_recv(),
            Ok(UiToWorkerCommand::SetArtistGroupCollapsed {
                id: 10,
                collapsed: true
            })
        ));

        state.dispatch(FxAction::EditGroup(10), &core_tx, &fx_tx, 0.0);
        assert!(state.group_editor.is_some());
        assert!(state.has_open_modal());
        assert!(state.close_top_modal());
        assert!(state.group_editor.is_none());
    }
}
