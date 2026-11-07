# PROGRESS-FX

## Current Phase: F0 -> F1

### Phase F0: Workspace split, pure refactor
- [x] 0.1 Root workspace with `[workspace.dependencies]` and `[workspace.lints]`.
- [x] 0.2 Move code per §2 move map (`bcvault-core` and `bcvault`).
- [x] 0.3 Add §2 core API surface (`spawn_worker` with `RepaintHook`, `fetch_release_raw`, `fetch_bytes`, `Db::open_with`, `acquire_instance_lock`) and call instance lock from classic `main`.
- [x] 0.4 `scripts/check.sh` runs `--workspace` and updated grep guard paths.
- [x] Milestone verified: all workspace tests pass, clippy/fmt clean.

### Phase F1: Foundations
- [ ] 1.1 `bcvault-fx` skeleton, `FxApp`, two-receiver event drain, `FxAction` dispatcher.
- [ ] 1.2 Sidecar DB (`fx.sqlite`), migrations, reader with ATTACH, contract check, `Prefs`.
- [ ] 1.3 `motion/` (springs, OKLab, `Motion`) with tests.
- [ ] 1.4 Theme system (4 themes, transitions), fonts & OS fallback, icons, `paint/` helpers, squareness tests & grep guard.
- [ ] 1.5 F3 overlay, `EffectsLevel` and `reduced_motion` plumbing.

### Phase F2: Thumbnail and palette pipeline
- [ ] 2.1 Capture task & `docs/endpoints.md`.
- [ ] 2.2 Tier derivation (`T64`, `T192`, `T384`, `Hero`), disk cache, `TextureCache` LRU, priority queue & generations.
- [ ] 2.3 Palette extraction (median cut in OKLab), `tiny8`, persistence, startup preload.
- [ ] 2.4 Placeholder rendering (`tiny8` 8x8 mosaic + geometric fallback) and fade-in widget `paint::cover`.

### Phase F3: Cover Wall
- [ ] 3.1 `QueryWall` with chunked streaming and `FxQuery`; layout math.
- [ ] 3.2 Card rendering, hover/parallax/lift, selection, focus frame.
- [ ] 3.3 FLIP engine, entrances, ghosts; card-size change.
- [ ] 3.4 Groups, sticky headers, scrub rail; marquee selection; keyboard navigation.

### Phase F4: Table, sidebar, toolbar, dock
- [ ] 4.1 Table port with thumbnails and sliding highlight.
- [ ] 4.2 Sidebar with mosaics, smart views, saved views, and collapse.
- [ ] 4.3 Toolbar facets and colour filter.
- [ ] 4.4 Dock, progress springs, sparkline, toasts, gate chip.

### Phase F5: Drawer, navigation, palette
- [ ] 5.1 Drawer with hero tier and slide transitions.
- [ ] 5.2 `DetailService` (lazy 400ms dwell, deduplicated) and `release_detail`.
- [ ] 5.3 `nav.rs` history, breadcrumbs, saved views.
- [ ] 5.4 Command palette (`Ctrl+K`/`Ctrl+P`) with scopes (`@`, `#`, `>`), recents, and thumbnails.

### Phase F6: Artists and Stats views
- [ ] 6.1 Artists banners and shared-element dive.
- [ ] 6.2 Squarified treemap, 53x7 heat-calendar, yearly bars, quota gauges, odometer.

### Phase F7: Atmosphere and polish
- [ ] 7.1 Grain + vignette overlay, ambient backdrop, scanline tile, compile-failure fallback.
- [ ] 7.2 Adaptive accent.
- [ ] 7.3 Micro-polish pass (`busy_squares`, empty states, hover states).
- [ ] 7.4 `examples/fx_bench.rs`.

### Phase F8: Hardening and packaging
- [ ] 8.1 Windows extended paths in cover cache, cache purge & size readout, settings export, docs/README.
