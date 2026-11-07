use crate::{
    actions::{FxQuery, FxSortKey, SmartView, ViewMode},
    motion::EffectsLevel,
    nav::{SavedView, ViewState},
    theme::ThemeId,
};
use bcvault_core::{
    db::{Db, Role, repo::releases::escape_like},
    error::DbError,
    types::{
        AudioFormat, Classification, ItemKey, ItemType, ReleaseFilter, ReleaseRow, SortDir, Ts,
        UpdateReason,
    },
};
use rusqlite::{Connection, OptionalExtension, params, types::Value};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

pub const FX_MIGRATIONS: &[(i32, &str)] = &[(1, include_str!("../../migrations/0001_fx_init.sql"))];

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prefs {
    pub theme: ThemeId,
    pub effects: EffectsLevel,
    pub reduced_motion: bool,
    pub adaptive_accent: bool,
    pub animated_grain: bool,
    pub grain_strength: f32,
    pub vignette_strength: f32,
    pub scanlines: bool,
    pub view: ViewMode,
    pub card_size_idx: usize,
    pub tex_budget_mb: u32,
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            theme: ThemeId::Carbon,
            effects: EffectsLevel::Full,
            reduced_motion: false,
            adaptive_accent: true,
            animated_grain: false,
            grain_strength: 0.03,
            vignette_strength: 0.22,
            scanlines: false,
            view: ViewMode::Wall,
            card_size_idx: 1,
            tex_budget_mb: 192,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Palette {
    pub art_id: i64,
    pub tiny8: [u8; 192],
    pub swatches: [[u8; 3]; 5],
    pub accent: [u8; 3],
    pub hue: Option<u16>,
    pub chroma: u8,
    pub luma: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub struct WallRow {
    pub rel: ReleaseRow,
    pub art_id: Option<i64>,
    pub hue: Option<u16>,
    pub accent: Option<[u8; 3]>,
    pub added_at: Ts,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Facets {
    pub all: u32,
    pub free: u32,
    pub nyp: u32,
    pub email_required: u32,
    pub paid: u32,
    pub not_downloaded: u32,
    pub updated: u32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Stats {
    pub total_releases: u32,
    pub downloaded_releases: u32,
    pub eligible_releases: u32,
    pub total_bytes: u64,
    pub by_class: [(Classification, u32); 5],
    pub artist_cells: Vec<(i64, String, Classification, u32)>,
    pub daily_downloads: Vec<(String, u32, u64)>,
    pub yearly_releases: Vec<(i32, Classification, u32)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackEntry {
    pub track_num: u32,
    pub title: String,
    pub duration_s: Option<u32>,
    pub stream_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
pub struct ReleaseDetail {
    pub fetched_at: Ts,
    pub tracks: Vec<TrackEntry>,
    pub about: Option<String>,
    pub credits: Option<String>,
    pub tags: Vec<String>,
    pub duration_s: Option<u32>,
}

#[derive(Clone)]
pub struct FxDb {
    pub writer: Db,
    pub reader: Db,
}

impl FxDb {
    pub fn open(fx_path: &Path, main_db_path: &Path) -> Result<Self, DbError> {
        let writer = Db::open_with(fx_path, Role::Writer, "", &[], FX_MIGRATIONS, false)?;
        let attach = vec![(main_db_path.to_path_buf(), "m".to_string())];
        let reader = Db::open_with(fx_path, Role::Reader, "", &attach, FX_MIGRATIONS, false)?;
        Ok(Self { writer, reader })
    }
}

const REQUIRED_MAIN_TABLES: &[(&str, &[&str])] = &[
    (
        "releases",
        &[
            "item_type",
            "item_id",
            "band_id",
            "title",
            "artist_name",
            "url",
            "cover_url",
            "art_id",
            "classification",
            "is_preorder",
            "track_count",
            "published_at",
            "modified_at",
            "first_seen_at",
            "downloaded",
            "downloaded_at",
            "downloaded_format",
            "local_path",
            "has_remote_update",
            "remote_update_reason",
        ],
    ),
    ("artists", &["band_id", "slug", "name", "url"]),
    (
        "download_history",
        &[
            "id",
            "item_type",
            "item_id",
            "band_id",
            "format",
            "status",
            "downloaded_at",
            "bytes_size",
        ],
    ),
];

pub fn verify_main_schema_contract(conn: &Connection) -> Result<(), DbError> {
    for &(table, expected_cols) in REQUIRED_MAIN_TABLES {
        let sql = format!("PRAGMA m.table_info({table})");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
        let mut found = HashSet::new();
        for r in rows {
            found.insert(r?);
        }
        if found.is_empty() {
            return Err(DbError::SchemaMismatch(format!(
                "missing table 'm.{table}' in shared database"
            )));
        }
        for &col in expected_cols {
            if !found.contains(col) {
                return Err(DbError::SchemaMismatch(format!(
                    "missing column 'm.{table}.{col}' in shared database (schema mismatch, update bcvault-fx)"
                )));
            }
        }
    }
    Ok(())
}

pub fn get_prefs(conn: &Connection) -> Result<Prefs, DbError> {
    let mut prefs = Prefs::default();
    let mut stmt = conn.prepare("SELECT key, value FROM prefs")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows {
        let (k, v) = row?;
        match k.as_str() {
            "theme" => {
                if let Some(t) = ThemeId::parse(&v) {
                    prefs.theme = t;
                }
            }
            "effects" => {
                if let Some(e) = EffectsLevel::parse(&v) {
                    prefs.effects = e;
                }
            }
            "reduced_motion" => prefs.reduced_motion = v == "1" || v == "true",
            "adaptive_accent" => prefs.adaptive_accent = v == "1" || v == "true",
            "animated_grain" => prefs.animated_grain = v == "1" || v == "true",
            "grain_strength" => {
                if let Ok(f) = v.parse::<f32>() {
                    prefs.grain_strength = f.clamp(0.0, 0.06);
                }
            }
            "vignette_strength" => {
                if let Ok(f) = v.parse::<f32>() {
                    prefs.vignette_strength = f.clamp(0.0, 0.30);
                }
            }
            "scanlines" => prefs.scanlines = v == "1" || v == "true",
            "view" => {
                if let Some(vm) = ViewMode::parse(&v) {
                    prefs.view = vm;
                }
            }
            "card_size_idx" => {
                if let Ok(idx) = v.parse::<usize>() {
                    prefs.card_size_idx = idx.min(3);
                }
            }
            "tex_budget_mb" => {
                if let Ok(mb) = v.parse::<u32>() {
                    prefs.tex_budget_mb = mb.clamp(64, 1024);
                }
            }
            _ => {}
        }
    }
    Ok(prefs)
}

pub fn set_pref(conn: &Connection, key: &str, value: &str) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO prefs (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

pub fn upsert_cover_meta(conn: &Connection, p: &Palette) -> Result<(), DbError> {
    let mut swatch_bytes = Vec::with_capacity(15);
    for rgb in &p.swatches {
        swatch_bytes.extend_from_slice(rgb);
    }
    let accent_int =
        ((i64::from(p.accent[0])) << 16) | ((i64::from(p.accent[1])) << 8) | i64::from(p.accent[2]);
    conn.execute(
        "INSERT INTO cover_meta (art_id, tiny8, swatches, accent_rgb, hue, chroma, luma, src_fetched_at, failed_at, fail_count)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, unixepoch(), NULL, 0)
         ON CONFLICT(art_id) DO UPDATE SET
             tiny8 = excluded.tiny8,
             swatches = excluded.swatches,
             accent_rgb = excluded.accent_rgb,
             hue = excluded.hue,
             chroma = excluded.chroma,
             luma = excluded.luma,
             src_fetched_at = unixepoch(),
             failed_at = NULL,
             fail_count = 0",
        params![
            p.art_id,
            p.tiny8.as_slice(),
            swatch_bytes,
            accent_int,
            p.hue.map(i64::from),
            i64::from(p.chroma),
            i64::from(p.luma),
        ],
    )?;
    Ok(())
}

pub fn record_cover_failure(conn: &Connection, art_id: i64) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO cover_meta (art_id, failed_at, fail_count)
         VALUES (?1, unixepoch(), 1)
         ON CONFLICT(art_id) DO UPDATE SET
             failed_at = unixepoch(),
             fail_count = cover_meta.fail_count + 1",
        params![art_id],
    )?;
    Ok(())
}

pub fn is_cover_in_negative_cache(conn: &Connection, art_id: i64) -> Result<bool, DbError> {
    let failed_at: Option<i64> = conn
        .query_row(
            "SELECT failed_at FROM cover_meta WHERE art_id = ?1 AND fail_count > 0",
            params![art_id],
            |r| r.get(0),
        )
        .optional()?
        .flatten();
    if let Some(ts) = failed_at {
        let now = chrono::Utc::now().timestamp();
        return Ok(now.saturating_sub(ts) < 86_400);
    }
    Ok(false)
}

pub fn load_all_palettes(conn: &Connection) -> Result<Vec<Palette>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT art_id, tiny8, swatches, accent_rgb, hue, chroma, luma
         FROM cover_meta
         WHERE tiny8 IS NOT NULL",
    )?;
    let rows = stmt.query_map([], |r| {
        let art_id: i64 = r.get(0)?;
        let tiny_blob: Vec<u8> = r.get(1)?;
        let swatch_blob: Option<Vec<u8>> = r.get(2)?;
        let accent_int: Option<i64> = r.get(3)?;
        let hue: Option<u16> = r.get(4)?;
        let chroma: Option<u8> = r.get(5)?;
        let luma: Option<u8> = r.get(6)?;

        let mut tiny8 = [0_u8; 192];
        if tiny_blob.len() == 192 {
            tiny8.copy_from_slice(&tiny_blob);
        }
        let mut swatches = [[0_u8; 3]; 5];
        if let Some(sb) = swatch_blob
            && sb.len() >= 15
        {
            for i in 0..5 {
                swatches[i] = [sb[i * 3], sb[i * 3 + 1], sb[i * 3 + 2]];
            }
        }
        let acc = accent_int.unwrap_or(0x00_D1_B2);
        let accent = [
            ((acc >> 16) & 0xFF) as u8,
            ((acc >> 8) & 0xFF) as u8,
            (acc & 0xFF) as u8,
        ];
        Ok(Palette {
            art_id,
            tiny8,
            swatches,
            accent,
            hue,
            chroma: chroma.unwrap_or(50),
            luma: luma.unwrap_or(50),
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn get_release_detail(
    conn: &Connection,
    key: ItemKey,
) -> Result<Option<ReleaseDetail>, DbError> {
    let row = conn
        .query_row(
            "SELECT fetched_at, tracks_json, about, credits, tags_json, duration_s
             FROM release_detail
             WHERE item_type = ?1 AND item_id = ?2",
            params![key.kind.as_str(), key.id],
            |r| {
                let fetched_at: i64 = r.get(0)?;
                let tracks_str: Option<String> = r.get(1)?;
                let about: Option<String> = r.get(2)?;
                let credits: Option<String> = r.get(3)?;
                let tags_str: Option<String> = r.get(4)?;
                let duration_s: Option<u32> = r.get(5)?;
                let tracks: Vec<TrackEntry> = tracks_str
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or_default();
                let tags: Vec<String> = tags_str
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or_default();
                Ok(ReleaseDetail {
                    fetched_at,
                    tracks,
                    about,
                    credits,
                    tags,
                    duration_s,
                })
            },
        )
        .optional()?;
    Ok(row)
}

pub fn upsert_release_detail(
    conn: &Connection,
    key: ItemKey,
    detail: &ReleaseDetail,
) -> Result<(), DbError> {
    let tracks_json = serde_json::to_string(&detail.tracks)?;
    let tags_json = serde_json::to_string(&detail.tags)?;
    conn.execute(
        "INSERT INTO release_detail (item_type, item_id, fetched_at, tracks_json, about, credits, tags_json, duration_s)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT(item_type, item_id) DO UPDATE SET
             fetched_at = excluded.fetched_at,
             tracks_json = excluded.tracks_json,
             about = excluded.about,
             credits = excluded.credits,
             tags_json = excluded.tags_json,
             duration_s = excluded.duration_s",
        params![
            key.kind.as_str(),
            key.id,
            detail.fetched_at,
            tracks_json,
            detail.about,
            detail.credits,
            tags_json,
            detail.duration_s.map(i64::from),
        ],
    )?;
    Ok(())
}

pub fn list_saved_views(conn: &Connection) -> Result<Vec<SavedView>, DbError> {
    let mut stmt =
        conn.prepare("SELECT id, name, state_json, pos FROM saved_views ORDER BY pos ASC, id ASC")?;
    let rows = stmt.query_map([], |r| {
        let id: i64 = r.get(0)?;
        let name: String = r.get(1)?;
        let json: String = r.get(2)?;
        let pos: i32 = r.get(3)?;
        let state: ViewState = serde_json::from_str(&json).unwrap_or_default();
        Ok(SavedView {
            id,
            name,
            state,
            pos,
        })
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn save_view(conn: &Connection, name: &str, state: &ViewState) -> Result<SavedView, DbError> {
    let json = serde_json::to_string(state)?;
    let next_pos: i32 = conn.query_row(
        "SELECT COALESCE(MAX(pos), 0) + 1 FROM saved_views",
        [],
        |r| r.get(0),
    )?;
    conn.execute(
        "INSERT INTO saved_views (name, state_json, pos) VALUES (?1, ?2, ?3)",
        params![name, json, next_pos],
    )?;
    let id = conn.last_insert_rowid();
    Ok(SavedView {
        id,
        name: name.to_string(),
        state: state.clone(),
        pos: next_pos,
    })
}

pub fn delete_saved_view(conn: &Connection, id: i64) -> Result<(), DbError> {
    conn.execute("DELETE FROM saved_views WHERE id = ?1", params![id])?;
    Ok(())
}

pub fn touch_recent(conn: &Connection, kind: &str, r#ref: &str) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO recents (kind, ref, ts) VALUES (?1, ?2, unixepoch())
         ON CONFLICT(kind, ref) DO UPDATE SET ts = unixepoch()",
        params![kind, r#ref],
    )?;
    Ok(())
}

pub fn list_recents(conn: &Connection, limit: usize) -> Result<Vec<(String, String)>, DbError> {
    let mut stmt = conn.prepare("SELECT kind, ref FROM recents ORDER BY ts DESC LIMIT ?1")?;
    let rows = stmt.query_map(params![limit as i64], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub const fn fx_order_by_clause(sort: FxSortKey, dir: SortDir) -> &'static str {
    match (sort, dir) {
        (FxSortKey::Released, SortDir::Desc) => "r.published_at DESC, r.item_type, r.item_id",
        (FxSortKey::Released, SortDir::Asc) => "r.published_at ASC, r.item_type, r.item_id",
        (FxSortKey::Added, SortDir::Desc) => "r.first_seen_at DESC, r.item_type, r.item_id",
        (FxSortKey::Added, SortDir::Asc) => "r.first_seen_at ASC, r.item_type, r.item_id",
        (FxSortKey::Artist, SortDir::Asc) => {
            "artist_name COLLATE NOCASE ASC, r.title COLLATE NOCASE ASC, r.item_type, r.item_id"
        }
        (FxSortKey::Artist, SortDir::Desc) => {
            "artist_name COLLATE NOCASE DESC, r.title COLLATE NOCASE DESC, r.item_type, r.item_id"
        }
        (FxSortKey::Title, SortDir::Asc) => "r.title COLLATE NOCASE ASC, r.item_type, r.item_id",
        (FxSortKey::Title, SortDir::Desc) => "r.title COLLATE NOCASE DESC, r.item_type, r.item_id",
        (FxSortKey::Type, SortDir::Asc) => "r.item_type ASC, r.published_at DESC, r.item_id",
        (FxSortKey::Type, SortDir::Desc) => "r.item_type DESC, r.published_at DESC, r.item_id",
        (FxSortKey::Tracks, SortDir::Asc) => "r.track_count ASC, r.item_type, r.item_id",
        (FxSortKey::Tracks, SortDir::Desc) => "r.track_count DESC, r.item_type, r.item_id",
        (FxSortKey::Classification, SortDir::Asc) => "r.classification ASC, r.item_type, r.item_id",
        (FxSortKey::Classification, SortDir::Desc) => {
            "r.classification DESC, r.item_type, r.item_id"
        }
        (FxSortKey::Format, SortDir::Asc) => "r.downloaded_format ASC, r.item_type, r.item_id",
        (FxSortKey::Format, SortDir::Desc) => "r.downloaded_format DESC, r.item_type, r.item_id",
        (FxSortKey::Status, SortDir::Asc) => {
            "r.downloaded ASC, r.has_remote_update ASC, r.item_type, r.item_id"
        }
        (FxSortKey::Status, SortDir::Desc) => {
            "r.downloaded DESC, r.has_remote_update DESC, r.item_type, r.item_id"
        }
        (FxSortKey::Hue, SortDir::Asc) => {
            "CASE WHEN c.hue IS NULL THEN 1 ELSE 0 END ASC, c.hue ASC, r.item_type, r.item_id"
        }
        (FxSortKey::Hue, SortDir::Desc) => {
            "CASE WHEN c.hue IS NULL THEN 1 ELSE 0 END ASC, c.hue DESC, r.item_type, r.item_id"
        }
    }
}

pub fn build_wall_query_sql(query: &FxQuery) -> (String, Vec<Value>) {
    let mut where_parts: Vec<&'static str> = Vec::new();
    let mut values: Vec<Value> = Vec::new();

    if let Some(band_id) = query.base.band_id {
        where_parts.push("r.band_id = ?");
        values.push(Value::Integer(band_id));
    }

    if let Some(smart) = query.smart {
        match smart {
            SmartView::RecentlyAdded => {}
            SmartView::FreeNotDownloaded => {
                where_parts.push("r.downloaded = 0 AND r.classification IN ('free','nyp')");
            }
            SmartView::Updated => where_parts.push("r.has_remote_update = 1"),
            SmartView::Preorders => where_parts.push("r.is_preorder = 1"),
            SmartView::NeedsMetadata => where_parts.push("r.classification = 'unknown'"),
        }
    } else {
        match query.base.filter {
            ReleaseFilter::All => {}
            ReleaseFilter::Free => where_parts.push("r.classification = 'free'"),
            ReleaseFilter::Nyp => where_parts.push("r.classification = 'nyp'"),
            ReleaseFilter::EmailRequired => where_parts.push("r.classification = 'email_required'"),
            ReleaseFilter::Paid => where_parts.push("r.classification = 'paid'"),
            ReleaseFilter::NotDownloaded => where_parts.push(
                "r.downloaded = 0 AND r.classification IN ('free','nyp','email_required','unknown')",
            ),
            ReleaseFilter::Updated => where_parts.push("r.has_remote_update = 1"),
        }
    }

    if let Some((h_lo, h_hi)) = query.hue {
        where_parts.push("c.hue IS NOT NULL AND c.hue BETWEEN ? AND ?");
        values.push(Value::Integer(i64::from(h_lo)));
        values.push(Value::Integer(i64::from(h_hi)));
    }

    if let Some((y_lo, y_hi)) = query.years {
        let ts_lo = chrono::NaiveDate::from_ymd_opt(y_lo, 1, 1)
            .and_then(|d| d.and_hms_opt(0, 0, 0))
            .map(|dt| dt.and_utc().timestamp())
            .unwrap_or(0);
        let ts_hi = chrono::NaiveDate::from_ymd_opt(y_hi, 12, 31)
            .and_then(|d| d.and_hms_opt(23, 59, 59))
            .map(|dt| dt.and_utc().timestamp())
            .unwrap_or(i64::MAX);
        where_parts.push("r.published_at BETWEEN ? AND ?");
        values.push(Value::Integer(ts_lo));
        values.push(Value::Integer(ts_hi));
    }

    if query.only_with_art {
        where_parts.push("r.art_id IS NOT NULL");
    }

    let trimmed = query.base.search.trim();
    if !trimmed.is_empty() {
        let pattern = format!("%{}%", escape_like(trimmed));
        where_parts.push("(r.title LIKE ? ESCAPE '!' OR r.artist_name LIKE ? ESCAPE '!')");
        values.push(Value::Text(pattern.clone()));
        values.push(Value::Text(pattern));
    }

    let where_sql = if where_parts.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_parts.join(" AND "))
    };

    let effective_sort = if query.smart == Some(SmartView::RecentlyAdded) {
        FxSortKey::Added
    } else {
        query.fx_sort
    };
    let order_sql = fx_order_by_clause(effective_sort, query.base.dir);

    let join_kw = if query.hue.is_some() {
        "INNER JOIN"
    } else {
        "LEFT JOIN"
    };
    let sql = format!(
        "SELECT r.item_type, r.item_id, r.band_id, r.title, r.artist_name, r.url, r.cover_url, r.art_id,
                r.classification, r.is_preorder, r.track_count, r.published_at, r.modified_at, r.first_seen_at,
                r.downloaded, r.downloaded_at, r.downloaded_format, r.local_path,
                r.has_remote_update, r.remote_update_reason, c.hue, c.accent_rgb
         FROM m.releases r
         {join_kw} cover_meta c ON c.art_id = r.art_id
         {where_sql}
         ORDER BY {order_sql}"
    );
    (sql, values)
}

pub fn query_wall(conn: &Connection, query: &FxQuery) -> Result<Vec<WallRow>, DbError> {
    let (sql, values) = build_wall_query_sql(query);
    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(values.iter()), |r| {
        let t_str: String = r.get(0)?;
        let kind = ItemType::parse(&t_str).unwrap_or(ItemType::Album);
        let id: i64 = r.get(1)?;
        let band_id: i64 = r.get(2)?;
        let title: String = r.get(3)?;
        let artist_name: String = r.get(4)?;
        let url: String = r.get(5)?;
        let cover_url: Option<String> = r.get(6)?;
        let art_id: Option<i64> = r.get(7)?;
        let class_str: String = r.get(8)?;
        let classification = Classification::parse(&class_str).unwrap_or(Classification::Unknown);
        let is_preorder: i64 = r.get(9)?;
        let track_count: Option<u32> = r.get(10)?;
        let published_at: Option<Ts> = r.get(11)?;
        let modified_at: Option<Ts> = r.get(12)?;
        let first_seen_at: Ts = r.get(13)?;
        let downloaded: i64 = r.get(14)?;
        let downloaded_at: Option<Ts> = r.get(15)?;
        let fmt_str: Option<String> = r.get(16)?;
        let downloaded_format = fmt_str.as_deref().and_then(AudioFormat::parse);
        let local_path_str: Option<String> = r.get(17)?;
        let local_path = local_path_str.map(PathBuf::from);
        let has_remote_update: i64 = r.get(18)?;
        let reason_str: Option<String> = r.get(19)?;
        let remote_update_reason = reason_str.as_deref().and_then(UpdateReason::parse);
        let hue: Option<u16> = r.get(20)?;
        let accent_int: Option<i64> = r.get(21)?;
        let accent = accent_int.map(|acc| {
            [
                ((acc >> 16) & 0xFF) as u8,
                ((acc >> 8) & 0xFF) as u8,
                (acc & 0xFF) as u8,
            ]
        });

        Ok(WallRow {
            rel: ReleaseRow {
                key: ItemKey { kind, id },
                band_id,
                title,
                artist_name,
                url,
                cover_url,
                art_id,
                classification,
                is_preorder: is_preorder != 0,
                track_count,
                published_at,
                modified_at,
                downloaded: downloaded != 0,
                downloaded_at,
                downloaded_format,
                local_path,
                has_remote_update: has_remote_update != 0,
                remote_update_reason,
            },
            art_id,
            hue,
            accent,
            added_at: first_seen_at,
        })
    })?;

    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

pub fn query_facets(conn: &Connection, query: &FxQuery) -> Result<Facets, DbError> {
    let mut where_parts: Vec<&'static str> = Vec::new();
    let mut values: Vec<Value> = Vec::new();

    if let Some(band_id) = query.base.band_id {
        where_parts.push("band_id = ?");
        values.push(Value::Integer(band_id));
    }

    let trimmed = query.base.search.trim();
    if !trimmed.is_empty() {
        let pattern = format!("%{}%", escape_like(trimmed));
        where_parts.push("(title LIKE ? ESCAPE '!' OR artist_name LIKE ? ESCAPE '!')");
        values.push(Value::Text(pattern.clone()));
        values.push(Value::Text(pattern));
    }

    let where_sql = if where_parts.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_parts.join(" AND "))
    };

    let sql = format!(
        "SELECT classification, COUNT(*), COALESCE(SUM(downloaded = 0), 0), COALESCE(SUM(has_remote_update), 0)
         FROM m.releases
         {where_sql}
         GROUP BY classification"
    );

    let mut stmt = conn.prepare(&sql)?;
    let rows = stmt.query_map(rusqlite::params_from_iter(values.iter()), |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, u32>(1)?,
            r.get::<_, u32>(2)?,
            r.get::<_, u32>(3)?,
        ))
    })?;

    let mut facets = Facets::default();
    for r in rows {
        let (class_str, count, not_dl, upd) = r?;
        facets.all += count;
        facets.updated += upd;
        match Classification::parse(&class_str) {
            Some(Classification::Free) => {
                facets.free += count;
                facets.not_downloaded += not_dl;
            }
            Some(Classification::Nyp) => {
                facets.nyp += count;
                facets.not_downloaded += not_dl;
            }
            Some(Classification::EmailRequired) => {
                facets.email_required += count;
                facets.not_downloaded += not_dl;
            }
            Some(Classification::Paid) => {
                facets.paid += count;
            }
            Some(Classification::Unknown) => {
                facets.not_downloaded += not_dl;
            }
            None => {}
        }
    }
    Ok(facets)
}

pub fn query_stats(conn: &Connection) -> Result<Stats, DbError> {
    let mut stats = Stats {
        by_class: [
            (Classification::Free, 0),
            (Classification::Nyp, 0),
            (Classification::EmailRequired, 0),
            (Classification::Paid, 0),
            (Classification::Unknown, 0),
        ],
        ..Default::default()
    };

    let (tot, dl, elig): (u32, u32, u32) = conn.query_row(
        "SELECT COUNT(*),
                COALESCE(SUM(downloaded), 0),
                COALESCE(SUM(classification IN ('free','nyp','email_required')), 0)
         FROM m.releases",
        [],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    stats.total_releases = tot;
    stats.downloaded_releases = dl;
    stats.eligible_releases = elig;

    let total_bytes: i64 = conn.query_row(
        "SELECT COALESCE(SUM(bytes_size), 0) FROM m.download_history WHERE status = 'success'",
        [],
        |r| r.get(0),
    )?;
    stats.total_bytes = total_bytes.max(0) as u64;

    {
        let mut stmt = conn.prepare(
            "SELECT r.band_id, COALESCE(a.name, r.artist_name), r.classification, COUNT(*)
             FROM m.releases r
             LEFT JOIN m.artists a ON a.band_id = r.band_id
             GROUP BY r.band_id, r.classification",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, u32>(3)?,
            ))
        })?;
        for r in rows {
            let (bid, name, c_str, cnt) = r?;
            let cl = Classification::parse(&c_str).unwrap_or(Classification::Unknown);
            for (slot_c, slot_cnt) in &mut stats.by_class {
                if *slot_c == cl {
                    *slot_cnt += cnt;
                }
            }
            stats.artist_cells.push((bid, name, cl, cnt));
        }
    }

    {
        let mut stmt = conn.prepare(
            "SELECT strftime('%Y-%m-%d', downloaded_at, 'unixepoch', 'localtime') AS d,
                    COUNT(*),
                    COALESCE(SUM(bytes_size), 0)
             FROM m.download_history
             WHERE status = 'success' AND downloaded_at >= unixepoch('now', '-370 days')
             GROUP BY d
             ORDER BY d ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, u32>(1)?,
                r.get::<_, i64>(2)?,
            ))
        })?;
        for r in rows {
            let (d_opt, cnt, b) = r?;
            if let Some(d) = d_opt {
                stats.daily_downloads.push((d, cnt, b.max(0) as u64));
            }
        }
    }

    {
        let mut stmt = conn.prepare(
            "SELECT CAST(strftime('%Y', published_at, 'unixepoch') AS INTEGER) AS y,
                    classification,
                    COUNT(*)
             FROM m.releases
             WHERE published_at IS NOT NULL
             GROUP BY y, classification
             ORDER BY y ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, Option<i32>>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, u32>(2)?,
            ))
        })?;
        for r in rows {
            let (y_opt, c_str, cnt) = r?;
            if let Some(y) = y_opt {
                let cl = Classification::parse(&c_str).unwrap_or(Classification::Unknown);
                stats.yearly_releases.push((y, cl, cnt));
            }
        }
    }

    Ok(stats)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use bcvault_core::db::{DbPool, seed};
    use std::time::Instant;

    #[tokio::test]
    async fn fx_migrations_and_readonly_attach() {
        let dir = tempfile::tempdir().unwrap();
        let main_path = dir.path().join("bcvault.db");
        let fx_path = dir.path().join("fx.sqlite");

        let _main_pool = DbPool::open(&main_path).unwrap();
        let fx_db = FxDb::open(&fx_path, &main_path).unwrap();

        fx_db
            .reader
            .call(|c| {
                verify_main_schema_contract(c)?;
                // Attempting to write to attached main DB must fail (read-only)
                let write_res = c.execute(
                    "INSERT INTO m.artists (band_id, slug, name, url) VALUES (1, 'a', 'A', 'https://a.bandcamp.com')",
                    [],
                );
                assert!(write_res.is_err());
                Ok(())
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn explain_query_plan_wall_and_hue() {
        let dir = tempfile::tempdir().unwrap();
        let main_path = dir.path().join("bcvault.db");
        let fx_path = dir.path().join("fx.sqlite");

        let _main_pool = DbPool::open(&main_path).unwrap();
        let fx_db = FxDb::open(&fx_path, &main_path).unwrap();

        fx_db
            .reader
            .call(|c| {
                let mut q = FxQuery::default();
                q.base.band_id = Some(10_000);
                let (sql, _) = build_wall_query_sql(&q);
                let explain_sql = format!("EXPLAIN QUERY PLAN {sql}");
                let mut stmt = c.prepare(&explain_sql)?;
                let rows = stmt.query_map([10_000_i64], |r| r.get::<_, String>(3))?;
                let mut plan_lines = Vec::new();
                for r in rows {
                    plan_lines.push(r?);
                }
                let joined = plan_lines.join(" | ");
                assert!(
                    joined.contains("idx_releases_band_pub"),
                    "expected idx_releases_band_pub in plan, got: {joined}"
                );
                assert!(!joined.contains("SCAN r"), "unexpected SCAN r: {joined}");

                let q_hue = FxQuery {
                    hue: Some((120, 180)),
                    ..Default::default()
                };
                let (sql_h, _) = build_wall_query_sql(&q_hue);
                let explain_h = format!("EXPLAIN QUERY PLAN {sql_h}");
                let mut stmt_h = c.prepare(&explain_h)?;
                let rows_h = stmt_h.query_map([120_i64, 180_i64], |r| r.get::<_, String>(3))?;
                let mut plan_h = Vec::new();
                for r in rows_h {
                    plan_h.push(r?);
                }
                let joined_h = plan_h.join(" | ");
                assert!(
                    joined_h.contains("idx_cover_hue")
                        || joined_h.contains("cover_meta")
                        || joined_h.contains("SEARCH c"),
                    "expected cover_meta/idx_cover_hue in plan, got: {joined_h}"
                );
                Ok(())
            })
            .await
            .unwrap();
    }

    #[test]
    fn contract_check_detects_missing_column() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "ATTACH DATABASE ':memory:' AS m;
             CREATE TABLE m.releases (item_type TEXT, item_id INTEGER);",
        )
        .unwrap();
        let err = verify_main_schema_contract(&conn).unwrap_err();
        assert!(matches!(err, DbError::SchemaMismatch(_)));
    }

    #[tokio::test]
    async fn seeded_100k_wall_query_under_budget() {
        let dir = tempfile::tempdir().unwrap();
        let main_path = dir.path().join("bcvault.db");
        let fx_path = dir.path().join("fx.sqlite");

        let main_pool = DbPool::open(&main_path).unwrap();
        main_pool
            .writer
            .call(|c| seed::seed(c, 100_000))
            .await
            .unwrap();

        let fx_db = FxDb::open(&fx_path, &main_path).unwrap();
        fx_db
            .reader
            .call(|c| {
                let mut q = FxQuery::default();
                q.base.band_id = Some(1);
                let t0 = Instant::now();
                let rows = query_wall(c, &q)?;
                let elapsed = t0.elapsed();
                assert_eq!(rows.len(), 500);
                assert!(
                    elapsed.as_millis() < 60,
                    "per-artist wall query took {} ms (budget 60 ms)",
                    elapsed.as_millis()
                );
                Ok(())
            })
            .await
            .unwrap();
    }
}
