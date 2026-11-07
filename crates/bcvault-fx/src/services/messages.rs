use crate::{
    actions::FxQuery,
    db::{Facets, Palette, Prefs, ReleaseDetail, Stats, WallRow},
    nav::{SavedView, ViewState},
};
use bcvault_core::types::ItemKey;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tier {
    T64,
    T192,
    T384,
    Hero,
}

impl Tier {
    pub const fn edge_px(self) -> u32 {
        match self {
            Self::T64 => 64,
            Self::T192 => 192,
            Self::T384 => 384,
            Self::Hero => 700,
        }
    }

    pub const fn suffix(self) -> &'static str {
        match self {
            Self::T64 => "t64",
            Self::T192 => "t192",
            Self::T384 => "t384",
            Self::Hero => "hero",
        }
    }

    pub fn for_display_px(px: f32) -> Self {
        if px <= 64.0 {
            Self::T64
        } else if px <= 192.0 {
            Self::T192
        } else {
            Self::T384
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SearchScope {
    #[default]
    All,
    Artists,
    Filters,
    Commands,
}

#[derive(Clone, Debug)]
pub enum SearchHitKind {
    Artist { band_id: i64 },
    Release { key: ItemKey, art_id: Option<i64> },
    Filter(bcvault_core::types::ReleaseFilter),
    Command(crate::actions::FxAction, &'static str),
}

#[derive(Clone, Debug)]
pub struct SearchHit {
    pub title: String,
    pub subtitle: String,
    pub match_indices: Vec<usize>,
    pub score: i64,
    pub kind: SearchHitKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Warn,
    Error,
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub id: u64,
    pub kind: ToastKind,
    pub message: String,
    pub created_s: f64,
    pub undo_batch_id: Option<u64>,
}

#[derive(Clone, Debug)]
pub enum FxCommand {
    QueryWall {
        request_id: u64,
        query: FxQuery,
    },
    QueryFacets {
        request_id: u64,
        query: FxQuery,
    },
    QueryStats {
        request_id: u64,
    },
    Search {
        request_id: u64,
        text: String,
        scope: SearchScope,
    },
    EnsureThumbs {
        generation: u64,
        items: Vec<(i64, Tier, u32)>,
    },
    EnsureHero {
        art_id: i64,
    },
    EnsureDetail {
        key: ItemKey,
    },
    SaveView {
        name: String,
        state: ViewState,
    },
    DeleteView(i64),
    SetPref {
        key: String,
        value: String,
    },
    TouchRecent {
        kind: String,
        r#ref: String,
    },
    PurgeCovers,
    Shutdown,
}

#[derive(Clone, Debug)]
pub enum FxEvent {
    Ready {
        prefs: Prefs,
        saved_views: Vec<SavedView>,
        palettes: Vec<Palette>,
        recents: Vec<(String, String)>,
        schema_error: Option<String>,
        cover_cache_bytes: u64,
    },
    WallLoaded {
        request_id: u64,
        chunk: u32,
        last: bool,
        rows: Vec<WallRow>,
    },
    FacetsLoaded {
        request_id: u64,
        facets: Facets,
    },
    StatsLoaded {
        request_id: u64,
        stats: Stats,
    },
    SearchResults {
        request_id: u64,
        hits: Vec<SearchHit>,
    },
    ThumbReady {
        art_id: i64,
        tier: Tier,
        image: Arc<egui::ColorImage>,
    },
    PaletteReady {
        art_id: i64,
        palette: Palette,
    },
    DetailLoaded {
        key: ItemKey,
        detail: Box<ReleaseDetail>,
    },
    SavedViewsUpdated(Vec<SavedView>),
    CoverCachePurged,
    ThumbFailed {
        art_id: i64,
    },
    Notice {
        kind: ToastKind,
        message: String,
    },
}
