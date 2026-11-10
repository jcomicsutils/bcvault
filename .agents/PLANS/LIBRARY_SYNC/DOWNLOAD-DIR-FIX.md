# Plan: keep the download directory out of artist links and stop false local-only albums

For the coding agent. Repo: bcvault (Rust workspace). Scope: `crates/bcvault-core/src/worker/{library,supervisor}.rs`, `crates/bcvault-core/src/library/service.rs`, docs.

> Status 10 Nov 2026: **planned**; will land on `main` as `Task: none` commits. Amends `LIBRARY_SYNC_PLAN.md` §5 and `MULTI_FOLDER.md`.

## 0. Ground rules

Same as `MATCH-FIX.md` §0: stay on `main`, one commit per step in section 4, every commit builds and passes `scripts/check.sh`, no `unwrap`/`expect` outside tests, never modify or delete user files, no migration.

## 1. Problem

Reported flow:

1. The user downloads an artist's releases with the app's own download function.
2. Later they move the albums from the download directory into their music library and link that library folder to the artist.
3. Every album in the library is reported as **Local Only**, and the sidebar badge counts them as non-eligible downloads.

Before step 3 the download directory also behaved as if it were tied to the artist page: the releases pointed into it, nothing showed it as a link, and there was nothing to remove.

## 2. Causes found by reading the code

1. **Wrong download directory in the link guard.** `LibraryService` reads settings with `get_settings(c, PathBuf::from("downloads"))`. While the user has never saved settings, the real default download directory (`AppPaths::default_download_dir`, for example `~/Music/bcvault`) comes only from the supervisor boot read. The overlap check in `lib_repo::add_folder` therefore compares a candidate folder with the relative path `downloads`, which exists nowhere, and the download directory passes validation. The same fallback is used by the resolve, verify, rescan and match paths, so their `out_dir` attribution is wrong too.
2. **Releases downloaded by the app can never be adopted.** All three matching entry points (`library/service.rs::run_scan_and_match`, `LibraryService::rescan_and_adopt_on_sync`, `LibraryService::match_artist_folder_internal`) only offer releases with `downloaded = 0` or `local_status = archived` to `assign_matches`. A release downloaded into the download directory has `downloaded = 1` and a `local_path` there, so it is treated as already matched. The library copy of the same album has no release to match and becomes a local-only orphan, which also raises `artist_folders.local_only_count`.

## 3. Goals and non-goals

Goals:

- The download directory (equal to, inside, or containing a folder) can never be linked, whether or not settings were saved.
- An album in a linked folder matches a release that was downloaded outside every linked folder, using the normal exact-first matching, and adoption re-points the release to the library copy.
- Such releases never show up as On Page Only while they are still downloaded.

Non-goals:

- No change to manual `downloaded` marks without a path, to archived releases, or to releases inside a linked folder.
- No automatic removal of the files left behind in the download directory (rule 8 of `AGENTS.md`).
- No migration and no change to `validate_folder_path` itself; it was correct and only received the wrong directory.

## 4. Steps

1. `fix(library): guard the real download dir when linking folders` (`Task: none`). `LibraryService` gets `default_out_dir` and `with_default_out_dir`; every production settings read goes through one `settings_reader()`; the supervisor passes `AppPaths::default_download_dir`. Test: with no saved settings, linking the download directory and a folder inside it both fail and leave no `artist_folders` row.
2. `fix(library): match library albums to releases downloaded elsewhere` (`Task: none`). New `service::split_matchable_releases` is the single place that decides which releases are adoptable: `downloaded = 0`, archived, or downloaded with a `local_path` outside every linked folder of the artist (unavailable folders included, so a disconnected drive never frees its releases). The three entry points call it, and `drop_downloaded_page_only` drops still-downloaded releases from `page_only`. Tests: library album matches an app download, the report has no local-only and no page-only entry, and the count in `artist_folders` stays 0; a release inside a linked folder is untouched.
3. Docs ship with each commit: `docs/library.md` §2 and §4, and `PROGRESS.md`.

## 5. Verification

- `scripts/check.sh` green.
- Manual (ask the user): download one free release with the app, move the album folder into a library folder, link that folder, and check that the album appears under Matched (or is adopted automatically) with no Local Only entry and no `(+1)` in the sidebar badge. Then try linking the download directory: it must be refused.
