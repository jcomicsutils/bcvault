use crate::motion::EffectsLevel;
use bcvault_core::types::{ItemKey, ReleaseFilter, ReleaseQuery, SortDir, SyncMode};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ViewMode {
    #[default]
    Wall,
    Table,
    Artists,
    Stats,
}

impl ViewMode {
    pub const ALL: [Self; 4] = [Self::Wall, Self::Table, Self::Artists, Self::Stats];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Wall => "Wall",
            Self::Table => "Table",
            Self::Artists => "Artists",
            Self::Stats => "Stats",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Wall => "wall",
            Self::Table => "table",
            Self::Artists => "artists",
            Self::Stats => "stats",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "wall" => Some(Self::Wall),
            "table" => Some(Self::Table),
            "artists" => Some(Self::Artists),
            "stats" => Some(Self::Stats),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GroupBy {
    #[default]
    None,
    Year,
    Artist,
    Classification,
    Initial,
}

impl GroupBy {
    pub const ALL: [Self; 5] = [
        Self::None,
        Self::Year,
        Self::Artist,
        Self::Classification,
        Self::Initial,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "No grouping",
            Self::Year => "By Year",
            Self::Artist => "By Artist",
            Self::Classification => "By Class",
            Self::Initial => "By Initial (A–Z)",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum FxSortKey {
    #[default]
    Released,
    Added,
    Artist,
    Title,
    Type,
    Tracks,
    Classification,
    Format,
    Status,
    Hue,
}

impl FxSortKey {
    pub const ALL: [Self; 10] = [
        Self::Released,
        Self::Added,
        Self::Artist,
        Self::Title,
        Self::Type,
        Self::Tracks,
        Self::Classification,
        Self::Format,
        Self::Status,
        Self::Hue,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Released => "Released",
            Self::Added => "Added",
            Self::Artist => "Artist",
            Self::Title => "Title",
            Self::Type => "Type",
            Self::Tracks => "Tracks",
            Self::Classification => "Class",
            Self::Format => "Format",
            Self::Status => "Status",
            Self::Hue => "Colour Hue",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SmartView {
    RecentlyAdded,
    FreeNotDownloaded,
    Updated,
    Preorders,
    NeedsMetadata,
    Archived,
    MissingFiles,
}

impl SmartView {
    pub const ALL: [Self; 7] = [
        Self::RecentlyAdded,
        Self::FreeNotDownloaded,
        Self::Updated,
        Self::Preorders,
        Self::NeedsMetadata,
        Self::Archived,
        Self::MissingFiles,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::RecentlyAdded => "Recently added",
            Self::FreeNotDownloaded => "Free & not downloaded",
            Self::Updated => "Updated remotely",
            Self::Preorders => "Pre-orders",
            Self::NeedsMetadata => "Needs metadata",
            Self::Archived => "Archived",
            Self::MissingFiles => "Missing files",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FxQuery {
    pub base: ReleaseQuery,
    pub fx_sort: FxSortKey,
    pub group: GroupBy,
    pub hue: Option<(u16, u16)>,
    pub years: Option<(i32, i32)>,
    pub only_with_art: bool,
    pub smart: Option<SmartView>,
}

impl Default for FxQuery {
    fn default() -> Self {
        Self {
            base: ReleaseQuery::default(),
            fx_sort: FxSortKey::Released,
            group: GroupBy::None,
            hue: None,
            years: None,
            only_with_art: false,
            smart: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum FxAction {
    SetView(ViewMode),
    OpenPalette,
    ClosePalette,
    FocusSearch,
    SetFilter(ReleaseFilter),
    SetSort(FxSortKey, SortDir),
    SetGroup(GroupBy),
    SetHue(Option<(u16, u16)>),
    SelectSmartView(Option<SmartView>),
    SelectArtist(Option<i64>),
    OpenDrawer(ItemKey),
    CloseDrawer,
    DrawerStep(i32),
    ToggleSelect(ItemKey),
    SelectRange(ItemKey),
    ClearSelection,
    DownloadSelected,
    DownloadKey(ItemKey),
    OpenInBrowser(ItemKey),
    OpenFolder(ItemKey),
    RedeemDialog(ItemKey),
    SetReleaseDownloaded(ItemKey, bool),
    SetReleaseUnavailable(ItemKey, bool),
    RelinkReleaseModal(ItemKey),
    SyncSelected(SyncMode),
    SyncAll(SyncMode),
    CardSize(i8),
    ToggleSidebar,
    SetTheme(String),
    ReloadThemes,
    OpenThemesFolder,
    SetAdaptiveAccent(bool),
    SetEffects(EffectsLevel),
    SetReducedMotion(bool),
    NavBack,
    NavForward,
    SaveCurrentView(String),
    ApplySavedView(i64),
    DeleteSavedView(i64),
    PurgeCovers,
    ToggleDebug,
    OpenLinkFolderModal(Option<i64>),
    VerifyFiles,
    UndoBatch(u64),
}
