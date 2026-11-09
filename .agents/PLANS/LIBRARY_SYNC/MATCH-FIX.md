# Plan: exact-first folder matching with a user-declared folder style

For the coding agent. Repo: bcvault (Rust workspace). Scope: `crates/bcvault-core/src/library/*`, the `Settings` struct, both UI settings modals, and docs.

> Status 8 Nov 2026: **implemented** on `main` (eight `Task: none` commits, listed in `COMMIT_PLAN.md` section U). The `sync library` branch named below no longer exists; the work was committed on `main`.

## 0. Ground rules

- Stay on the current branch (it is already the `sync library` branch). Do not create or switch branches, rebase, or force-push.
- Use Conventional Commits, one commit per step in section 6. Every commit must compile and pass tests. Subject line 72 characters or less, imperative mood, body explains why.
- Before each commit run `scripts/check.sh` (fmt, clippy with `-D warnings`, tests, UI primitive bans). Non-test code must not use `unwrap` or `expect`.
- Library sync is local-only I/O. Never modify, rename, move or delete user files. Do not change `scan.rs` or `tags.rs`.
- Keep these public shapes unchanged so the worker and UIs need no changes beyond the settings editor: `MatchAssignment`, `ReconcileReport`, `MatchProposal`, `ReviewItem`, `Conflict`, `Score`, `Signal`.
- Edition 2024, MSRV 1.89.
- Read before editing: `library/{matcher,textmatch,verify,service,scan}.rs`, `worker/library.rs` (grep `assign_matches` and `find_relink_candidates`), `Settings` in `types.rs`, `db/repo/settings.rs`, `bandcamp/filenames.rs` (reuse placeholder-parsing helpers only if natural; do not couple that setting to this one), the settings modals in `crates/bcvault/src/ui/modals.rs` and `crates/bcvault-fx/src/overlays/modals.rs`, and `tests/sync_pipeline.rs`.
- Line numbers in this plan come from a snapshot. Grep, do not trust them.

## 1. Problem

Reported case: artist `Leichen`, releases like `Leiche 75 / Leiche 76`, local folders like `Leiche 75 - Leiche 76`. Every release in the series was reported as a Conflict listing 17 unrelated sibling folders.

Causes found by reading the code (the scores below came from a Python port of the formulas, so first reproduce the failure with a Rust test before fixing anything):

1. The fallback compares album tags with normalized Levenshtein. `leiche 73 leiche 74` vs `leiche 75 leiche 76` scores 0.895. Equal track count adds 0.05 and equal year adds 0.02, giving 0.945 to 0.965, above the 0.92 auto threshold.
2. `assign_matches` step 1 declares a Conflict when the top two scores are within 0.05, then lists every candidate at or above `review_match_min`. An exact match (1.0) next to a near-miss (0.965) is flagged as a conflict.
3. The folder title is taken with `rsplit_once(" - ")`, which cuts `Leiche 75 - Leiche 76` down to `Leiche 76`.
4. `strip_noise_tokens` computes byte offsets once and reuses them after the string shrinks. `Album [FLAC] [WEB]` panics. This function is being deleted anyway.
5. `find_relink_candidates` in `verify.rs` has the same runner-up flaw.

## 2. Goals and non-goals

Goals:

- An exact match is always decisive.
- A Conflict only ever comes from true duplicates.
- Numbered series (Vol 1 / Vol 2, Leiche 73 / Leiche 75) can never be confused with each other.
- Accent-insensitive comparison.
- The user declares how their folders are named (one global setting).

Non-goals:

- Nothing is ever stripped implicitly (`[FLAC]`, `(320)`, trailing years). Users declare their decorations in the folder style, through the noise list and the `{noise}` placeholder.
- No NFKC, no `&` to `and` mapping, no per-artist styles, no style auto-detection.
- No DB schema change, no network, no change to scanning or verification of files on disk.

## 3. Specification

### 3.1 Name keys (`textmatch.rs`)

`pub fn tokens(s: &str) -> Vec<String>`, applied in this order:

1. Lowercase (`str::to_lowercase`).
2. Canonical decomposition (NFD) using the `unicode-normalization` crate, then drop combining marks.
3. Map letters that do not decompose: ø to o, ł to l, đ to d, ð to d, ħ to h, ı to i, ß to ss, æ to ae, œ to oe, þ to th.
4. Delete apostrophes: U+0027, U+2019, U+2018, U+02BC (so `Don't` becomes `dont`).
5. Every other non-alphanumeric character becomes a separator (`char::is_alphanumeric`; this keeps CJK, Cyrillic and so on).
6. Split on separators and discard empty tokens.

`pub fn key(s: &str) -> String` is the tokens joined by a single space.

Empty keys: titles made only of symbols (`!!!`, `△`) have an empty key. Two empty keys must never compare equal. When `key(title)` is empty, compare `trim().to_lowercase()` of the raw strings instead.

Add `unicode-normalization = "0.1"` to `[workspace.dependencies]` and to `bcvault-core/Cargo.toml`. Check `.github/workflows/ci.yml` for any license or audit step the new dependency must pass.

### 3.2 Folder style

```
pub struct FolderStyle {
    pub templates: Vec<String>,   // serde default: one template, {title}
    pub noise: Vec<String>,       // serde default: empty; used only by the {noise} placeholder
}
```

Template grammar: literal text plus the placeholders `{title}` (required, exactly once), `{artist}` (at most once), `{year}` (at most once) and `{noise}` (at most once). Reject unknown placeholders, unbalanced braces, empty templates and templates over 128 characters. Allow at most 8 templates; drop duplicates.

Parsed form: a list of segments `Title`, `Artist`, `Year`, `Noise`, `Literal(tokens(text))`. Literals with zero tokens are dropped. Only tokens matter, so punctuation and bracket style in a template are irrelevant: `{title} ({year})` also fits `Album [2019]` and `Album - 2019`.

Matching a template against `(folder_name, release.artist_name)`:

- Let `t = tokens(folder_name)`.
- `Artist`, `Year` and `Literal` have a fixed token length. `Artist` equals `tokens(artist_name)` (the template does not fit if that is empty). `Year` is exactly one token of four ASCII digits with value 1900 to 2099. `Literal` equals its tokens.
- `Noise` is the only variable-length segment besides `Title`: it is any one entry of the noise list, compared as a whole token sequence (never as a substring). Try the entries longest first (most tokens); the first one that fits at that position wins. With an empty noise list, a template containing `{noise}` never fits.
- Segments before `Title` consume tokens from the left, segments after it consume from the right, and `Title` is the remainder, which must be non-empty. The only branching is the choice of noise entry, so matching cost stays linear in the number of entries.
- Result: `Some(Extracted { title_key, year })` or `None`.

| Template | Noise list | Folder | Release artist | Result |
| --- | --- | --- | --- | --- |
| `{title}` | none | `Leiche 75 - Leiche 76` | Leichen | title `leiche 75 leiche 76` |
| `{artist} - {title}` | none | `Leiche 75 - Leiche 76` | Leichen | does not fit (first token `leiche` is not `leichen`) |
| `{artist} - {title}` | none | `Radiohead - Kid A` | Radiohead | title `kid a` |
| `{year} - {title}` | none | `1999 - Album` | any | title `album`, year 1999 |
| `{title} ({year})` | none | `Album [2019]` | any | title `album`, year 2019 |
| `{title} ({noise})` | FLAC, MP3 | `Album [FLAC]` | any | title `album` |
| `{title} ({noise})` | FLAC, MP3 | `Album (mp3)` | any | title `album` |
| `{title} ({noise})` | FLAC, MP3 | `Album [OGG]` | any | does not fit (OGG is not in the noise list) |
| `{title} ({noise})` | FLAC, WEB FLAC | `Album (WEB FLAC)` | any | title `album` (longest entry is tried first) |

Noise list: `FolderStyle.noise` holds the user's entries as typed (for example `FLAC`, `MP3`, `MP3 V0`, `24bit`, `WEB FLAC`). Each entry is tokenized with `tokens`, so case, accents and punctuation never matter, and an entry may span several tokens. Reject entries that produce no tokens, drop duplicates (compared by tokens), and allow at most 32 entries of 64 characters each. Noise is recognized only through the `{noise}` placeholder; nothing is stripped implicitly.

Validation API: `FolderStyle::validate(&self) -> Result<(), String>` covers grammar errors and invalid noise entries, with messages suitable for the UI. `FolderStyle::hints(&self) -> Vec<String>` returns non-blocking notes, for example that a template uses `{noise}` while the noise list is empty and so can never fit. Runtime code uses only valid templates and entries. If no template is valid it falls back to the default. Loading persisted settings must never fail because of a bad template or entry.

### 3.3 Names for a (candidate, release) pair

- Folder names: `key(candidate.name)` always (zips use the file stem, as today), plus the `title_key` of every template that fits. Remove duplicates.
- Tag name: `key(album tag)` if present. Tags are not templated.
- Extracted year: the year from the first fitting template that has one. It can feed the year signal in the fuzzy pass.

### 3.4 Exact pass (new, runs before any scoring)

An edge `(candidate, release)` exists when any of these hold:

- The tag URL equals the release URL (existing `normalize_bandcamp_url`). Signal `UrlTag`.
- Any folder name equals `key(release.title)`. Signal `FolderName`.
- The album tag key equals `key(release.title)`. Signal `AlbumTag`.

Signals are merged per pair. An edge becomes a `Score` of value 1.0 with those signals. Candidates in `ignored_path_keys` and candidates already linked are excluded before this pass (existing behavior).

Resolution:

1. **URL tags first.** A `UrlTag` edge is authoritative when its candidate has exactly one `UrlTag` edge and its release has exactly one `UrlTag` edge. Mark it matched, then remove both nodes and all other edges touching them.
2. **Components.** Build connected components over the remaining edges.
   - A component with a single edge is Matched. It is Restorable if `release.downloaded && local_status == Archived`, as today.
   - A component with more than one edge becomes exactly one `Conflict` listing only the candidates and releases in that component. Use a specific description: `N folders match release X exactly`, `Folder X matches N releases exactly`, or `Folder name matches X but album tag matches Y`.
3. Matched and conflicted nodes are excluded from the fuzzy pass.
4. Track count, year and runner-up margins play no role in this pass.

### 3.5 Fuzzy pass (only for what the exact pass left over)

Keep `score_candidate` and its structure, with these changes:

- Name similarity is the maximum over the folder names from 3.3 of `fuzzy_sim(name, key(title))`. Album similarity is `fuzzy_sim(tag key, key(title))`.
- `fuzzy_sim` is the existing `title_similarity` plus a **digit guard**: take every token that contains a numeric character; if the multisets of such tokens differ between the two names, return 0.0. The guard applies to the album-tag comparison as well, which was the Leiche failure path.
- Containment (one token set is a subset of the other, for example `Album` vs `Album Deluxe`) scores 0.85 and is **never auto-adopted**, regardless of the slider value or bonuses. Track `auto_eligible` in a private struct used inside the matrix; do not add a field to the public `Score`, because UI tests construct it with struct literals.
- Keep the track-count bonus and penalty, the year bonus, the 0.05 runner-up margin for auto adoption, and `review_match_min`.
- Delete `VERSION_WORDS`, `has_version_word_mismatch`, `extract_version_words`, the `rsplit_once(" - ")` logic, and `name_variant_keys`.
- Delete the old conflict-detection step from the fuzzy pass. Near-ties fail the margin check and appear in Review with all their candidates listed. Fuzzy matching never produces a `Conflict`.
- Keep the existing page-only and local-only logic.

### 3.6 Relink (`verify.rs`)

Put edge building and resolution in shared functions in `matcher.rs`, used by both `assign_matches` and `find_relink_candidates`. For missing releases:

- A single-edge exact component yields a `RelinkProposal` with score 1.0.
- A multi-edge component is unresolved (the user is asked), with the best candidate attached.
- Everything else goes through the existing fuzzy logic with the same margin and 1-to-1 claim rules.

### 3.7 Signatures

Add `style: &FolderStyle` to `assign_matches`, `score_release`, `score_candidate`, `find_relink_candidates` and `run_scan_and_match`. The worker passes `&st.folder_style` (the settings snapshot already used for `st.auto_match_min`). Update `library/mod.rs` re-exports.

## 4. Settings and UI

- In `types.rs` add `#[serde(default)] pub folder_style: FolderStyle` to `Settings` (the struct holds `templates` and `noise`, each with its own serde default) and set it in `default_with_dir`. Previously stored JSON must still deserialize. Fix any struct literals that stop compiling.
- On save (`set_settings`), drop invalid templates and noise entries, and fall back to the default template if none remain.
- In both UIs, add a **Folder naming** section next to the existing library settings (where the two threshold sliders are). It contains:
  - A list of editable template fields with a remove button each, and an Add button.
  - Preset buttons: `{title}`, `{artist} - {title}`, `{year} - {title}`, `{title} ({year})`, `{title} ({noise})`, `{artist} - {year} - {title}`.
  - A noise list editor: one text field per entry with a remove button, an Add button, and quick-add suggestions (FLAC, MP3, V0, 320, WAV, AAC, OGG, ALAC, 24bit). Entries are free text and the user owns the list.
  - Inline validation messages from `FolderStyle::validate`, and non-blocking hints from `FolderStyle::hints` (for example, a template uses `{noise}` but the noise list is empty).
  - Short help text: the placeholders, that punctuation is ignored, and that noise is only recognized through `{noise}`.
- Change the slider hover text to say the thresholds apply to non-exact (fuzzy) matches only.
- Optional, in a final separate commit: a Try it preview with fields for folder name, artist and title that shows which template fit, the extracted title, and whether it is an exact match.
- Follow the existing draft and save patterns. `scripts/check.sh` bans non-zero `corner_radius` in the classic UI and circles, rounding and Spinner in `bcvault-fx`.

## 5. Tests

Unit tests live next to the code. Write the Leiche regression first and confirm it fails on the current code before implementing.

| # | Scenario | Expected |
| --- | --- | --- |
| 1 | Leiche regression: artist `Leichen`, about 38 releases `Leiche 1 / Leiche 2` up to `Leiche 75 / Leiche 76` (step 2). Folders `Leiche X - Leiche Y`. Run three variants: album tag is the Bandcamp title, tag equals the folder name, no tag. Equal track counts and years. | All matched 1:1. Zero conflicts, zero review, zero local-only |
| 2 | Same as 1 but one release missing from the page | That folder is Local Only, not Conflict or Review (digit guard) |
| 3 | Same as 1 but one folder missing | That release is Page Only |
| 4 | Title with `/`, `:`, `?` vs folder with `-`, `_` or the characters removed | Exact match |
| 5 | Title `Foo - Bar`, style `{artist} - {title}`, folder `Artist - Foo - Bar`; also folder `Foo - Bar` | Both exact |
| 6 | Style `{artist} - {title}`, folder `Leiche 75 - Leiche 76`, artist `Leichen` | Template does not fit; raw folder name still matches exactly |
| 7 | Default style, folder `Album (2019)` vs title `Album` | No exact match; fuzzy containment gives Review at 0.85, not auto |
| 8 | Style `{title} ({year})`, folder `Album [2019]` | Exact; year 2019 extracted |
| 9 | Accents: `Sigur Rós` vs `Sigur Ros`; NFC vs NFD `Café`; `Björk`/`Bjork`; `Straße`/`Strasse`; `Ørnen`/`Ornen`; `Łódź`/`Lodz` | Exact |
| 10 | FLAC and MP3 folders both exact for one release, plus unrelated folders | One Conflict listing exactly those two folders |
| 11 | One folder exact for an album and a single with the same title | Conflict; with a URL tag on the folder, matched to that release |
| 12 | Folder name exact for release A, album tag exact for release B | Conflict with the disagreement description |
| 13 | Exact folder name, junk album tag | Matched |
| 14 | Releases `!!!` and `△`, folders `!!!` and `△` | Matched 1:1 by the raw fallback; never cross-matched |
| 15 | Fuzzy: `Volume 1` folder, only `Volume 2` on the page (and `Part 1` vs `Part 2`) | Not a candidate at all |
| 16 | `Album` folder vs `Album Deluxe` with equal track count and year, auto slider lowered to 0.80 | Review, never auto |
| 17 | Fuzzy near-tie between two candidates | Review lists both; no Conflict |
| 18 | Exact match on an archived, downloaded release | Goes to `restorable` |
| 19 | `ignored_path_keys` | Ignored candidates are skipped |
| 20 | Zip candidate whose stem equals the title | Exact |
| 21 | Relink: unique exact candidate; two exact candidates; fuzzy path unchanged | Relinked; unresolved; existing behavior. Adapt the 5 existing tests in `verify.rs` |
| 22 | `FolderStyle` parse and validate (valid, unknown placeholder, missing title, duplicate placeholder (including a second {noise}), unbalanced braces, length and count limits); template fitting incl. year bounds | As specified in 3.2 |
| 23 | `Settings` JSON without `folder_style` deserializes to the default; round trip | Passes |
| 24 | `textmatch` key tests: case, accents, special letters, apostrophes, separators, empty key | As specified in 3.1 |

Noise list tests (continuing the numbering above):

| # | Scenario | Expected |
| --- | --- | --- |
| 25 | Noise list FLAC and MP3, template `{title} ({noise})`, folders `Album [FLAC]`, `Album (mp3)` and `Album - FLAC` | All exact, title `album` |
| 26 | Same list, folder `Album [OGG]` | Template does not fit; fuzzy containment gives Review at 0.85, never auto |
| 27 | Noise entries `FLAC` and `WEB FLAC`, folder `Album (WEB FLAC)` | Longest entry wins; title `album` |
| 28 | Empty noise list with a `{noise}` template | Template never fits; the raw folder name still matches; `hints()` reports it |
| 29 | Release `Best Of MP3`, folder `Best Of MP3`, noise list contains MP3, template `{title} ({noise})` | Still exact through the raw folder name |
| 30 | Noise entry validation: empty after tokenizing, duplicates, over 32 entries, over 64 characters | Rejected or dropped as specified in 3.2 |
| 31 | Settings JSON that has `folder_style` without `noise` | Deserializes with an empty noise list |
| 32 | One artist folder mixing FLAC, MP3 and OGG releases, one template with `{noise}`, noise list FLAC and MP3 | FLAC and MP3 folders exact; the OGG folder falls to fuzzy as in 26 |

Also run the full workspace tests and grep `tests/sync_pipeline.rs` and `tests/download_pipeline.rs` for dependence on matching behavior or the old thresholds.

## 6. Commit sequence

Each step must leave the workspace compiling with all tests green.

1. `feat(library): add accent-insensitive name keys and folder style templates` adds `tokens`, `key`, `FolderStyle` (templates, noise list, parse, validate, hints, fit) and the dependency, alongside the old code, with tests 22, 24 and 25 to 30.
2. `feat(settings): persist folder style` adds the `Settings` field (templates and noise list), default, validation on save, and tests 23 and 31.
3. `fix(library): resolve exact matches before fuzzy scoring` covers `matcher.rs` per 3.4, 3.5 and 3.7. Includes the Leiche regression (written first) and tests 1 to 20 and 32. Updates `service.rs` and the worker call sites.
4. `fix(library): use exact-first matching when relinking missing albums` covers `verify.rs` per 3.6 and test 21.
5. `refactor(library): remove noise-token and diacritic-table text handling` deletes `strip_noise_tokens`, `strip_diacritics`, `name_variants`, `name_variant_keys`, the old `normalize` and their re-exports and tests.
6. `feat(ui): add folder naming settings to the classic UI` (templates and noise list).
7. `feat(fx): add folder naming settings to the fx UI` (templates and noise list).
8. `docs(library): document folder styles and exact-first matching` rewrites `docs/library.md` sections 3 and 4 (remove the NFKC and noise-stripping claims; describe the exact pass, the style templates, the noise list and what the thresholds now control). Grep `README.md` and modal strings for stale references to scoring.
9. Optional: `feat(ui): add folder style preview`.

## 7. Definition of done

- `scripts/check.sh` passes.
- The Leiche regression fails before step 3 and passes after.
- No exact match is ever turned into a Conflict by a non-exact candidate.
- `normalize` panics are gone because the code is gone.
- Old persisted settings load. The two sliders still work and only affect fuzzy matches.
- The only conflicts reported are the true-duplicate and disagreement cases in 3.4.

## 8. Decisions already made (do not re-litigate; flag if impossible)

- The default style is `{title}`. A library that uses `Artist - Title` folders must pick that preset.
- `&` is treated as a separator, not mapped to `and`.
- Apostrophes are deleted, not turned into spaces.
- A folder whose name matches one release while its album tag matches another is a Conflict with a specific description, not Review.
- Exact matches never use track count, year or runner-up margins.
- Noise is declared only through the noise list and the `{noise}` placeholder. Nothing is stripped implicitly, and a template that uses `{noise}` never fits while the noise list is empty.

If you discover something that makes the exact-first rule unworkable (for example a worker path that bypasses `assign_matches`), stop and report instead of improvising.
