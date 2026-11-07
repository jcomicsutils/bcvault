# PROGRESS-FX

## Current Phase: F8 (Complete)

### Phase F0: Workspace split, pure refactor
- [x] 0.1 Root workspace with `[workspace.dependencies]` and `[workspace.lints]`.
- [x] 0.2 Move code per §2 move map (`bcvault-core` and `bcvault`).
- [x] 0.3 Add §2 core API surface (`spawn_worker` with `RepaintHook`, `fetch_release_raw`, `fetch_bytes`, `Db::open_with`, `acquire_instance_lock`) and call instance lock from classic `main`.
- [x] 0.4 `scripts/check.sh` runs `--workspace` and updated grep guard paths.
- [x] Milestone verified: all workspace tests pass, clippy/fmt clean.

### Phase F1: Foundations
- [x] 1.1 `bcvault-fx` skeleton, `FxApp`, two-receiver event drain, `FxAction` dispatcher.
- [x] 1.2 Sidecar DB (`fx.sqlite`), migrations, reader with ATTACH, contract check, `Prefs`.
- [x] 1.3 `motion/` (springs, OKLab, `Motion`) with tests.
- [x] 1.4 Theme system (4 themes, transitions), fonts & OS fallback, icons, `paint/` helpers, squareness tests & grep guard.
- [x] 1.5 F3 overlay, `EffectsLevel` and `reduced_motion` plumbing.

### Phase F2: Thumbnail and palette pipeline
- [x] 2.1 Capture task & `docs/endpoints.md` (including `_0`, `_10`, `_16`, `_2`, `_7`).
- [x] 2.2 Tier derivation (`T64`, `T192`, `T384`, `Hero`), disk cache, `TextureCache` LRU, priority queue & generations.
- [x] 2.3 Palette extraction (median cut in OKLab), `tiny8`, persistence, startup preload.
- [x] 2.4 Placeholder rendering (`tiny8` 8x8 mosaic + geometric fallback) and fade-in widget `paint_cover_slot`.

### Phase F3: Cover Wall
- [x] 3.1 `QueryWall` with chunked streaming and `FxQuery`; layout math.
- [x] 3.2 Card rendering, hover/parallax/lift, selection, focus frame.
- [x] 3.3 FLIP engine, entrances, ghosts; card-size change.
- [x] 3.4 Groups, sticky headers, scrub rail; marquee selection; keyboard navigation.

### Phase F4: Table, sidebar, toolbar, dock
- [x] 4.1 Table port with thumbnails and sliding highlight.
- [x] 4.2 Sidebar with mosaics, smart views, saved views, and collapse.
- [x] 4.3 Toolbar facets and colour filter.
- [x] 4.4 Dock, progress springs, sparkline, toasts, gate chip.

### Phase F5: Drawer, navigation, palette
- [x] 5.1 Drawer with hero tier and slide transitions.
- [x] 5.2 `DetailService` (lazy 400ms dwell, deduplicated) and `release_detail`.
- [x] 5.3 `nav.rs` history, breadcrumbs, saved views.
- [x] 5.4 Command palette (`Ctrl+K`/`Ctrl+P`) with scopes (`@`, `#`, `>`), recents, and thumbnails.

### Phase F6: Artists and Stats views
- [x] 6.1 Artists banners and shared-element dive.
- [x] 6.2 Squarified treemap, 53x7 heat-calendar, yearly bars, quota gauges, odometer.

### Phase F7: Atmosphere and polish
- [x] 7.1 Grain + vignette overlay, ambient backdrop, scanline tile, compile-failure fallback.
- [x] 7.2 Adaptive accent.
- [x] 7.3 Micro-polish pass (`busy_squares`, empty states, hover states).
- [x] 7.4 `examples/fx_bench.rs`.

### Phase F8: Hardening and packaging
- [x] 8.1 Windows extended paths in cover cache, cache purge & size readout, settings export, docs/README.
