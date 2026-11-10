# AGENTS.md: standing instructions for the implementing agent

*Applies to the whole repository and to every plan. Read this first, then `ROADMAP.md`, then the plan you are executing. Prepared 7 Nov 2026, revised 8 Nov 2026 (synced with the codebase after L0 to L6, MATCH-FIX and the badge rework landed).*

## 1. Documents and precedence

| File | Role |
| --- | --- |
| `AGENTS.md` (this file) | Standing rules, commit convention, quality gate, workflow. Wins over every plan on process. |
| `ROADMAP.md` | Order of work across plans, dependencies, migration numbering, release plan, kickoff prompts. |
| `COMMIT_PLAN.md` | Every task ID with its exact commit subject. |
| `Initial Plan.md` | The base engine (Phases 0 to 5). Already implemented; its rules still apply. (Done)|
| `FX_PLAN.md` | `bcvault-fx` front-end (Phases F0 to F8). (Done)|
| `LIBRARY_SYNC/LIBRARY_SYNC_PLAN.md` | Artist folder links: one folder per artist, adopt and verify (Phases L0 to L6). (Done)|
| `LIBRARY_SYNC/MATCH-FIX.md` | Exact-first matching and folder style templates; amends `LIBRARY_SYNC_PLAN.md` §5 and §6. (Done, `Task: none` commits)|
| `LIBRARY_SYNC/MISSING-FOLDER-FIX.md` | Ask user when linked folder is gone; amends `LIBRARY_SYNC_PLAN.md` §5 and §6. (Done, `Task: none` commits)|
| `LIBRARY_SYNC/MULTI_FOLDER.md` | Several linked folders per artist; amends `LIBRARY_SYNC_PLAN.md` §5. (Done, `Task: none` commits) |
| `LIBRARY_SYNC/DOWNLOAD-DIR-FIX.md` | Keep the download dir out of artist links; match library albums to releases downloaded elsewhere; amends `LIBRARY_SYNC_PLAN.md` §5. (Done, `Task: none` commits)|
| `MAIL_REDEEM/MAIL_REDEEM_PLAN.md` | Mailbox redeem and cleanup (Phases M0 to M6). Not started. |
| `AUTO_SYNC/AUTO_SYNC.md` | Scheduled auto sync, quick check, removed-release tracking. Not started. |
| `ARCHIVE/ARCHIVE.md` | Selectable release archive (text and covers). Not started; merges after `AUTO_SYNC.md`. |
| `USER_THEMES/USER_THEMES.md` | File-based themes for `bcvault-fx`. Not started. |
| `PROGRESS.md`, `FX/PROGRESS-FX.md` | Live status. Wins over the status lines in every other document. |

Paths are relative to `.agents/` or `.agents/PLANS/`. `AUTO_SYNC.md`, `ARCHIVE.md`, `USER_THEMES.md`, `MATCH-FIX.md`, `MISSING-FOLDER-FIX.md`, `MULTI_FOLDER.md` and `DOWNLOAD-DIR-FIX.md` are step lists without phase IDs: their commits use `Task: none` and their step number goes in `PROGRESS.md`.

**Precedence:** the user's latest message, then this file, then the feature plan, then `Initial Plan.md`. Where a feature plan amends the base plan (for example the main-DB migrations `0004`/`0005` added by the library plan, which amend FX's "main schema untouched"), the amendment wins. Fixtures beat assumptions: if captured data disagrees with a plan's hypothesis about Bandcamp or an email format, follow the fixture and note it in `PROGRESS.md`.

## 2. The task loop

For every task ID in the plan you are executing:

1. **Read** the task, its milestone boxes and the matching row in `COMMIT_PLAN.md`. Update `PROGRESS.md` (`Current task`).
2. **Test first.** Write the failing test or fixture (parsers and SQL always; UI logic where it is pure). Bandcamp-facing code is developed against saved fixtures; the live site is touched only by capture tasks and the opt-in `live-tests` feature, and only after the user approved that capture.
3. **Implement** the smallest change that makes the test pass.
4. **Gate:** run `scripts/check.sh` (§4). Fix everything it reports. Never weaken a test, guard or lint to get green; if the plan itself is wrong, stop and follow §8.
5. **Commit** with the message from `COMMIT_PLAN.md` (§5). One task is one commit unless the table splits it into lettered commits.
6. **Log** in `PROGRESS.md` in the same commit (task ID in the `Done` list, new open questions, decisions).
7. At the end of a phase, run the milestone verification (every checkbox, including the manual ones: ask the user to perform manual checks you cannot do), tick the boxes in `PROGRESS.md`, and tag (§6).

## 3. Hard rules (consolidated from all plans)

1. `#![forbid(unsafe_code)]`. No `unwrap()`, `expect()` or `panic!` outside tests and `main` bootstrap; clippy enforces it.
2. **UI-thread purity.** `App::update` and all view/overlay code may only draw, read in-memory state, send commands and `try_recv` events. No `block_on`, `rusqlite`, `reqwest`, file I/O or blocking `rfd` dialogs (the one allowed exception is startup font loading in `fx/theme/fonts.rs`).
3. **One gate.** Every request to a Bandcamp host goes through `BandcampGate`. `reqwest::Client` is built only in `bandcamp/gate.rs`. IMAP traffic is not Bandcamp traffic, but every Bandcamp request the mail feature makes still uses the gate.
4. `scraper::Html` is `!Send`: parse inside plain synchronous functions that return owned structs; never hold an `Html` across `.await`.
5. **Square look.** Zero corner radius everywhere. FX paints only through `fx/paint` helpers. The grep guards in `scripts/guards.sh` must print nothing.
6. **Motion costs nothing at rest.** Springs stop requesting repaints when settled; every effect honours `EffectsLevel` and `reduced_motion`.
7. **Migrations are immutable once on `main`.** Changes go in a new numbered file. Number by landing order (`ROADMAP.md §5`); renaming an unmerged migration is allowed, editing a merged one is not.
8. **Never destroy information silently.** Library/status changes are batches with before/after events and undo. Never delete or modify user files; library code reads the filesystem only, and only inside linked artist folders and `out_dir`.
9. **Secrets never touch SQLite, settings, logs, errors or fixtures.** Passwords live in the OS keyring or in memory. `sig`/`payment_id`/`upn` values and recipient addresses are replaced in every committed fixture (§10).
10. All tests run offline (wiremock, fake sources, temp dirs). Anything touching the real internet or a real mailbox sits behind an opt-in feature and is never run by CI.
11. Versions in the plans are a known-good baseline. If an API has drifted, fix the call site rather than downgrading. All `egui*`/`eframe` crates share one minor version; check `cargo tree -d` after dependency changes.
12. Network, disk and mailbox politeness settings keep their conservative defaults unless the user changes them.
13. **SemVer and git tags.** Follow Semantic Versioning (maintain coherent version numbers: patch for fixes, minor for features/breaking migrations while on 0.x). Always create annotated SemVer git tags (`vMAJOR.MINOR.PATCH`) whenever releases or significant milestones land. Use version tags exclusively (no `milestone/<ID>` tags). Never leave releases untagged.
14. **Always update `PROGRESS.md`.** Every change, task completion, release bump, hotfix, or architectural deviation must update `PROGRESS.md` in the exact same commit. Never leave completed work undocumented or leave `PROGRESS.md` stale.

## 4. Quality gate

> Status (8 Nov 2026): only `scripts/check.sh` exists, and it is an older version (fmt, clippy, tests, two corner-radius guards). `guards.sh`, the commit checks, hooks, `.gitmessage` and `cliff.toml` from Appendix A are not installed yet (task T0, see `ROADMAP.md` §3). Until T0 lands, run the Appendix A commands by hand before each commit.

`scripts/check.sh` (full) and `scripts/check.sh --fast` (pre-commit) are defined in Appendix A.

| Gate | Commands | When |
| --- | --- | --- |
| Fast | `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `scripts/guards.sh` | pre-commit hook, after every edit loop |
| Full | Fast + `cargo test --workspace` | before every commit that is not trivially docs-only, and pre-push |
| Commit messages | `scripts/check-commit-msg.sh` (commit-msg hook), `scripts/check-commits.sh origin/main..HEAD` (pre-push, CI) | every commit |

**Every commit on every branch must build and pass the full gate.** There are no "WIP", "fix tests" or "make clippy happy" follow-up commits; amend or fixup into the task commit instead. Never use `--no-verify`.

## 5. Conventional Commits

Format (Conventional Commits 1.0.0, with a required scope and a `Task` trailer):

```text
<type>(<scope>)[!]: <imperative summary, lowercase, no period, max 72 chars>

<body: why the change exists and anything non-obvious, wrapped at 72>

Task: <ID>
[BREAKING CHANGE: <what breaks and how to recover>]
```

### 5.1 Types

| Type | Use for |
| --- | --- |
| `feat` | New user-visible or API-visible capability (including new tables and migrations) |
| `fix` | Bug fix (also changes to a plan's wrong assumption discovered by a fixture) |
| `perf` | Measured performance improvement; mention numbers |
| `refactor` | Behaviour-preserving restructuring (the F0 workspace move is `refactor`) |
| `test` | Tests, fixtures, sanitiser scripts, benchmark examples only |
| `docs` | Documentation and plan documents (`docs/`, READMEs, endpoint notes) |
| `build` | Cargo manifests, workspace layout, features, dependency changes that are not part of a feature commit |
| `ci` | Scripts, hooks, workflows, guards |
| `chore` | Housekeeping with no code effect (ignore files, licence headers) |
| `style` | Formatting-only changes (rare; `cargo fmt` is enforced) |

### 5.2 Scopes

Top-level scope is mandatory and validated; add `/sub` for a module. Allowed top-level scopes: `workspace`, `core`, `db`, `bandcamp`, `gate`, `worker`, `sync`, `downloads`, `quota`, `types`, `classic`, `fx`, `library`, `mail`, `tooling`, `deps`, `docs`, `plans`, `release`.

Examples of sub-scopes: `fx/motion`, `fx/theme`, `fx/paint`, `fx/thumbs`, `fx/wall`, `fx/table`, `fx/sidebar`, `fx/toolbar`, `fx/dock`, `fx/toasts`, `fx/drawer`, `fx/nav`, `fx/palette`, `fx/artists`, `fx/stats`, `fx/shader`, `fx/db`, `library/scan`, `mail/imap`, `mail/parse`, `mail/links`, `mail/cleanup`.

### 5.3 Trailers and footers

- `Task: <ID>` is required for `feat`, `fix`, `perf`, `refactor` and `test`. IDs are `B`/`F`/`L`/`M`/`T` + phase + `.` + task + optional letter (`F3.2b`, `L0.1`, `M4.3`, `T0.2`). Use `Task: none` for unplanned fixes. Repeat the trailer if a commit completes several tasks.
- `BREAKING CHANGE: ...` is required whenever the subject has `!`. Use `!` for anything that makes older binaries or data incompatible. Every main-DB migration that older binaries cannot open qualifies:

  ```text
  feat(db)!: add library status columns and artist folder link schema

  Adds migration 0004 with releases.local_status, artist_folders,
  artist_folder_ignored and library_batches/events.

  Task: L0.1
  BREAKING CHANGE: database schema v4; binaries built before this commit
  refuse to open it (TooNew). Rebuild and reinstall bcvault and
  bcvault-fx together.
  ```

- Optional `Refs:` footer for review notes, for example `Refs: MAIL_REDEEM_PLAN.md §6.2`.

### 5.4 Granularity and atomicity

- One task ID is one commit unless `COMMIT_PLAN.md` lists lettered commits. A commit changes one concern, builds, and passes the full gate.
- Keep a commit under about 400 changed lines excluding fixtures, generated files and `Cargo.lock`. If bigger, split along a seam (types, repo, service, UI).
- Fixtures go in their own `test(...)` commit when they are large (more than ~50 KB or more than 5 files); small fixtures ride with the code that uses them.
- A dependency is added in the commit that first uses it (manifest and `Cargo.lock` together). Pure bumps are `build(deps): bump <crate> to <version>`.
- Tests ship with the code they cover (same commit). "Tests first" means written first in your working loop, not committed red.
- Documentation of behaviour changes ships with the behaviour (same commit).

### 5.5 Examples

Good:

```text
feat(fx/wall): animate filter, sort and size changes with FLIP

Cards visible before and after a query change move from their old rect
to the new one with the SMOOTH spring, capped at 200 movers. New cards
enter staggered, leavers fade out as ghosts (max 40).

Task: F3.3
```

```text
fix(gate): honour Retry-After given as an HTTP date

Task: none
```

```text
test(mail/links): add hostile url table and malformed mime cases

Task: M1.5
```

Bad (all rejected by the hook): `Update stuff`, `feat: add wall` (no scope), `feat(fx/wall): Add wall.` (capital, period), `feat(wall): ...` (unknown scope), `feat(fx/wall): ...` without a `Task:` trailer, `wip`, `fix tests`.

### 5.6 History hygiene

- Linear history: rebase onto `main`, fast-forward merge, **no merge commits and no squash merges** (task-level commits are the record).
- `git commit --fixup` and `--autosquash` are fine locally; `fixup!`/`squash!` commits must be gone before push (`check-commits.sh` fails on them).
- Never force-push `main`. Force-pushing your own unmerged branch is fine after a rebase.
- Reverts use `git revert` and keep its generated subject, plus a `Task:` or `Refs:` line explaining why.

## 6. Branches, merging, tags

- Branch per phase: `phase/<PHASEID>-<slug>`, for example `phase/F3-cover-wall`, `phase/L0-library-model`. Hotfixes: `fix/<slug>`. Plans without phase IDs (`AUTO_SYNC.md`, `ARCHIVE.md`, `USER_THEMES.md`) use the `feat/<slug>` branch they name.
- Open a pull request (or ask the user to review) **per phase** once all milestone boxes pass. Then `git rebase origin/main` and fast-forward merge.
- Always follow SemVer and tag: after merge, create an annotated version tag `vMAJOR.MINOR.PATCH`. Never skip git tags when releasing versions or completing milestone phases.
- `main` is always green and shippable. No direct commits to `main` except `docs(plans)` and `chore` housekeeping approved by the user.

## 7. `PROGRESS.md`

**Mandatory rule: `PROGRESS.md` must ALWAYS be updated.** Every task, bugfix, chore, release bump, or decision must be recorded here in the same commit. Never leave completed work undocumented or leave `PROGRESS.md` stale.

Stored under `.agents/PROGRESS.md` (FX also keeps `.agents/FX/PROGRESS-FX.md`). Template:

```markdown
# PROGRESS

## Current
Task: F3.2b  ·  Branch: phase/F3-cover-wall  ·  Started: 2026-11-08

## Done
- B0.1 to B5.x: base engine (released v0.1.0)
- F0.1, F0.2a, F0.2b, ...

## Milestones
- [x] F0 (tag milestone/F0)
- [ ] F1: 3 of 7 boxes checked (list the unchecked ones)

## Open questions (needs the user)
- 2026-11-08: ... (blocking? yes/no)

## Decisions and deviations
- 2026-11-08: fixture shows X, plan assumed Y; followed fixture (commit: see `git log --grep "Task: F2.1"`)
```

Do not record commit hashes (they change on rebase); use the `Task:` trailer to find commits.

## 8. When to stop and ask

Stop and use `AskUserQuestion` (or write the question under *Open questions* and continue with other unblocked tasks) when:

- A plan step contradicts what the code, fixtures or tests show, and the fix costs more than half a day to redo.
- A choice listed under "Decisions made for you" or "Defaults assumed" needs to change.
- The next step needs something only the user has: sanitised `.eml` files, a live capture approval, an artist folder and page to link for a manual check, mailbox credentials, a manual verification.
- A milestone box cannot pass and the cause is outside the plan.

Otherwise decide, record it under *Decisions and deviations*, and continue. Never ask whether to proceed with an unambiguous task. Never touch the live Bandcamp site, a real mailbox or the user's real music folders outside an explicitly approved task.

## 9. Definition of done

**Task:** test written and passing, full gate green, behaviour documented where users or later agents need it, `PROGRESS.md` updated, commit message passes `check-commit-msg.sh`.

**Phase:** every milestone checkbox true (manual ones confirmed by the user), `scripts/check.sh` green on `main`, branch rebased and fast-forward merged, `milestone/<ID>` tag created, `PROGRESS.md` updated.

## 10. Fixtures, credentials and privacy

- Fixtures are sanitised before commit: replace recipient addresses, `sig`, `payment_id`, `upn` and cookies with same-length, same-class placeholders. Do not commit raw `.eml` files or raw HTTP captures. A sanitiser script is part of the capture task and has its own tests.
- Test mailboxes, passwords and tokens come from environment variables for opt-in live tests only and are never written to disk by the test.
- Logs carry counts and categories, never message text, passwords or signed links. `scripts/guards.sh` greps for the obvious mistakes; review is still required.
- Covers and downloaded files are personal-use data. Never commit them; the cover cache and download directories are git-ignored.

---

## Appendix A: scripts and configuration to create (task T0.x)

All scripts need `bash` and `ripgrep` (`rg`). On Windows use Git Bash. Make them executable.

### `scripts/check.sh`

```bash
#!/usr/bin/env bash
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
scripts/guards.sh
[[ "${1:-}" == "--fast" ]] && exit 0
cargo test --workspace
```

### `scripts/guards.sh`

```bash
#!/usr/bin/env bash
# Every guard command must print nothing. Add new guards here, never weaken one.
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
fail=0

guard() { # guard <description> <command...>
  local desc="$1"; shift
  local out; out="$("$@" 2>/dev/null || true)"
  if [[ -n "$out" ]]; then printf 'GUARD FAILED: %s\n%s\n\n' "$desc" "$out" >&2; fail=1; fi
}
no_round() { rg -n "$1" "${@:2}" | rg -v 'CornerRadius::ZERO'; }

UI_DIRS=(crates/bcvault/src/ui crates/bcvault-fx/src/views crates/bcvault-fx/src/overlays
         crates/bcvault-fx/src/paint crates/bcvault-fx/src/motion crates/bcvault-fx/src/theme)

guard "UI code must not block or do I/O" \
  rg -n 'block_on|rusqlite|reqwest|std::fs|tokio::fs|rfd::FileDialog' "${UI_DIRS[@]}" --glob '!**/fonts.rs'
guard "classic UI: non-zero corner radius" no_round 'corner_radius\(' crates/bcvault/src/ui
guard "fx: round primitives" no_round \
  'circle_|Shape::circle|rounding|corner_radius|CornerRadius::(same|from|new)|Spinner' crates/bcvault-fx/src
guard "reqwest clients must be built in bandcamp/gate.rs only" \
  rg -n 'reqwest::(Client|ClientBuilder)' crates --glob '!**/bandcamp/gate.rs' --glob '!**/tests/**' --glob '!**/examples/**'
guard "secrets in log macros" \
  rg -n -i '(info|warn|error|debug|trace)!\(.*(password|passwd|secret|[?&]sig=|\bsig\b)' crates
guard "shipped migrations must not be modified" bash -c '
  if git rev-parse --verify -q origin/main >/dev/null; then
    git diff --name-status origin/main...HEAD -- "crates/*/migrations/*.sql" | grep -v "^A" || true
  fi'

exit "$fail"
```

### `scripts/check-commit-msg.sh`

```bash
#!/usr/bin/env bash
# Validates one commit message file against AGENTS.md section 5.
set -euo pipefail

file="${1:?usage: check-commit-msg.sh <message-file>}"
mapfile -t lines < <(grep -v '^#' "$file")
subject="${lines[0]:-}"

fail() {
  printf 'commit-msg: %s\n  subject: %s\n  rules: AGENTS.md section 5\n' "$1" "$subject" >&2
  exit 1
}

# git-generated subjects and local autosquash markers (the latter are rejected by check-commits.sh)
if [[ "$subject" =~ ^Merge\  || "$subject" =~ ^Revert\ \" || "$subject" =~ ^(fixup|squash)!\  ]]; then
  exit 0
fi

scopes='workspace|core|db|bandcamp|gate|worker|sync|downloads|quota|types|classic|fx|library|mail|tooling|deps|docs|plans|release'
re="^(feat|fix|perf|refactor|test|docs|build|ci|chore|style)\\((${scopes})(/[a-z0-9-]+)?\\)(!)?: [a-z].*[^.]\$"

[[ "$subject" =~ $re ]] || fail "expected type(scope): lowercase imperative summary, no trailing period"
type="${BASH_REMATCH[1]}"
(( ${#subject} <= 72 )) || fail "subject is longer than 72 characters"
if (( ${#lines[@]} > 1 )) && [[ -n "${lines[1]}" ]]; then fail "second line must be blank"; fi

if [[ "$type" =~ ^(feat|fix|perf|refactor|test)$ ]]; then
  grep -Eq '^Task: ([BFLMT][0-9]+\.[0-9]+[a-z]?|none)$' "$file" \
    || fail "missing 'Task: <ID>' trailer (use 'Task: none' for unplanned work)"
fi
if [[ "$subject" == *')!:'* ]]; then
  grep -q '^BREAKING CHANGE: ' "$file" || fail "'!' requires a 'BREAKING CHANGE: ' footer"
fi
```

### `scripts/check-commits.sh`

```bash
#!/usr/bin/env bash
# Usage: scripts/check-commits.sh [range]   (default: origin/main..HEAD)
set -euo pipefail
range="${1:-origin/main..HEAD}"
tmp="$(mktemp)"; trap 'rm -f "$tmp"' EXIT
status=0

if [[ -n "$(git rev-list --merges "$range")" ]]; then
  echo "merge commits are not allowed (linear history): rebase instead" >&2; status=1
fi
while read -r sha; do
  subject="$(git log -1 --format=%s "$sha")"
  if [[ "$subject" =~ ^(fixup|squash)!\  ]]; then
    echo "unsquashed autosquash commit: $sha $subject" >&2; status=1; continue
  fi
  git log -1 --format=%B "$sha" > "$tmp"
  scripts/check-commit-msg.sh "$tmp" || { echo "  in commit $sha" >&2; status=1; }
done < <(git rev-list --reverse "$range")
exit "$status"
```

### Hooks (`.githooks/`) and installer

```bash
# .githooks/pre-commit
#!/usr/bin/env bash
exec scripts/check.sh --fast

# .githooks/commit-msg
#!/usr/bin/env bash
exec scripts/check-commit-msg.sh "$1"

# .githooks/pre-push
#!/usr/bin/env bash
set -euo pipefail
scripts/check.sh
if git rev-parse --verify -q origin/main >/dev/null; then scripts/check-commits.sh origin/main..HEAD; fi
```

```bash
# scripts/install-hooks.sh
#!/usr/bin/env bash
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
chmod +x .githooks/* scripts/*.sh
git config core.hooksPath .githooks
git config commit.template .gitmessage
echo "hooks installed"
```

### `.gitmessage`

```text
# type(scope): imperative summary, lowercase, max 72 chars, no trailing period
#
# Why this change exists (wrap at 72). Describe behaviour, not the diff.
#
# Task: F3.2b
# BREAKING CHANGE: only when the subject has '!'
```

### `cliff.toml` (changelog from commits with `git-cliff`; verify keys against its current docs)

```toml
[changelog]
header = "# Changelog\n"
body = """
{% for group, commits in commits | group_by(attribute="group") %}
### {{ group | upper_first }}
{% for commit in commits %}- {% if commit.scope %}**{{ commit.scope }}:** {% endif %}{{ commit.message | upper_first }}{% if commit.breaking %} **BREAKING**{% endif %}
{% endfor %}
{% endfor %}
"""
trim = true

[git]
conventional_commits = true
filter_unconventional = true
commit_parsers = [
  { message = "^feat", group = "Features" },
  { message = "^fix", group = "Fixes" },
  { message = "^perf", group = "Performance" },
  { message = "^refactor", group = "Refactoring" },
  { message = "^docs", group = "Documentation" },
  { message = "^(test|ci|build|chore|style)", skip = true },
]
```

### `.github/workflows/ci.yml` (only if the repository is hosted on GitHub; use current major versions of the actions, and add the system packages eframe needs on Linux)

```yaml
name: ci
on:
  push: { branches: [main] }
  pull_request:
jobs:
  check:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }
      - run: rustup show            # installs the toolchain pinned in rust-toolchain.toml
      - run: sudo apt-get update && sudo apt-get install -y ripgrep
      - run: scripts/check.sh
      - run: scripts/check-commits.sh origin/main..HEAD
```
