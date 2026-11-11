# Mailbox Redeem: fetch Bandcamp download links from your email on a button press

*Addendum to `Initial Plan.md` §6.9 (email-gated releases) · compatible with `FX_PLAN.md` and `LIBRARY_SYNC_PLAN.md` · lives in `bcvault-core`, UI in both front-ends · prepared 6 Nov 2026, revised 8 Nov 2026 (library plan is implemented; shared text matching is `bcvault_core::library::textmatch` plus `library::matcher`; migration number is the next free one, see `ROADMAP.md` §5)*

## 0. How to use this plan

**Goal.** For releases classified `EmailRequired`, the artist emails you a download link. You forward or receive those emails in an IMAP mailbox (your cock.li address). When you press **Check mail**, the app connects once, finds new Bandcamp download links, matches them to your library, and downloads them through the existing redeem pipeline, with no manual URL pasting. Nothing runs in the background: no polling, no IDLE, no always-on connection.

**Non-goals (deliberate)**
- The app does **not** submit your email address to artists' forms or otherwise automate the email gate. You still enter your address on Bandcamp yourself (the app can copy it to your clipboard). Bulk-subscribing you to artists' lists is exactly the bypass the base plan rules out.
- No sending mail, no deleting mail unless you turn on the cleanup option (§7.1), no reading mail that is not Bandcamp-related, no storing message bodies.

**Execution protocol.** Rules 2 to 7 of `Initial Plan.md §0` still apply (fixtures first, `forbid(unsafe_code)`, no `unwrap`/`expect`, UI-thread purity, one `BandcampGate` for every Bandcamp request, no `scraper::Html` across an `.await`). Additional rules:

1. IMAP traffic is **not** Bandcamp traffic, so it does not go through `BandcampGate`; but **every Bandcamp request this feature makes does** (download-page fetches and file downloads).
2. **The mailbox is treated as hostile input and as someone else's data**: read-only by default, fetch only what is needed, parse in memory, store only extracted links and minimal metadata.
3. **Secrets never touch SQLite, settings, logs, error messages or crash output.** A test asserts it (§13).
4. A user action (button press) is the only trigger. After a failed login the app does not retry on its own. If you enable email cleanup (§7.1), one short second session may run when that press's downloads finish; anything left over is retried only on your next press, never in the background.
5. Parsers are developed against owner-supplied, sanitised `.eml` files (§6.1). The first real sample has been received and is already reflected in this plan; where later fixtures disagree with the plan's assumptions, the fixtures win.

**Workflow.** Follow `AGENTS.md` (standing rules, Conventional Commits, quality gate) and `ROADMAP.md` (order and migration numbering). Task IDs for this plan are `M<phase>.<n>[letter]` (for example `M3.2`) or `Task: none` per phase/step; record completed tasks in `PROGRESS.md`.

**Decisions made for you (veto any)**

| Decision | Why |
| --- | --- |
| IMAP over implicit TLS (port 993) only in v1 | Simplest and safest; covers cock.li and most providers. STARTTLS is a stretch item. |
| Mailbox opened **read-only** (`EXAMINE`), messages fetched with `BODY.PEEK` | Checking mail never changes read flags or any state on the server. |
| Password in the OS keyring (Windows Credential Manager, macOS Keychain, Secret Service); if none is available, session-only in memory, never a file | No plaintext credential on disk, ever. |
| A **download mode** setting you pick: **Automatic** (matched, not-yet-downloaded items are enqueued right after the check) or **Review first** (a results list where you choose). Default Automatic; also switchable per check from the Check mail dropdown | Your call, either way, without digging through settings. |
| Optional **cleanup of processed emails** after successful downloads: leave / mark read / move to folder / move to Trash / delete permanently. Default: leave | Your request; the conservative rules are in §7.1. |
| Download-page fetches happen as part of the check, to learn which release each link is for | Matching needs the item id and title that only the download page contains. |

## 1. End-to-end flow

1. On an `EmailRequired` row: **Open in Browser** (existing), and a new **Copy my email address** button (the address you configured) so you can paste it into Bandcamp's form. The row gets a "waiting for email since 14:02" chip (`mail_wait`).
2. The artist's email lands in your cock.li mailbox (directly or forwarded).
3. You press **Check mail**. The app: connects, searches for new Bandcamp-related mail, parses it, extracts download links, validates them, fetches each link's download page through the gate, matches the items to releases.
4. Results: auto-enqueue (default) or review list. Downloads run through the normal queue, extraction and DB commit path (`source = 'redeem'`, `counts_against_quota = 0`).
5. Summary toast: "3 downloaded, 1 already downloaded, 1 expired, 2 not in your library".

## 2. Providers

| Provider | Settings | Notes |
| --- | --- | --- |
| **cock.li** (preset) | host `mail.cock.li`, port 993, TLS, username = full email address, normal password | Community-reported settings; the **Test connection** button is the authority. cock.li has had availability incidents, so the UI must show clear connection errors rather than hang. |
| Generic IMAP | host, port (default 993), TLS, username, password | Covers Fastmail, Posteo, mailbox.org and similar. |
| Gmail / Google Workspace | needs an app password (2-step verification on); OAuth is out of scope for v1 | Document in the dialog. |
| Microsoft (Outlook/Hotmail) | mostly OAuth-only | Unsupported in v1; suggest forwarding to an IMAP-password provider. |
| Proton Mail | via Proton Bridge on `127.0.0.1` (STARTTLS, self-signed certificate) | Stretch S1: STARTTLS plus certificate fingerprint pinning. |

## 3. Security design

| Concern | Design |
| --- | --- |
| Credential storage | `keyring` crate, entry `bcvault/mail/{account_id}`. Calls are blocking, so wrap in `spawn_blocking`. Load into `secrecy::SecretString`, zeroize after login. If the keyring is unavailable (headless Linux), prompt per session and keep in memory only. A "Forget password" button deletes the entry. |
| Transport | Implicit TLS with `tokio-rustls` and the platform-independent `webpki-roots`; certificate and hostname validation always on; no "ignore certificate errors" switch. Optional per-account SHA-256 certificate pin (for self-signed servers) is a stretch item. |
| Mailbox integrity | Check sessions use `EXAMINE` (read-only) and `UID FETCH ... BODY.PEEK[]`. The only session that opens the mailbox read-write is the optional **cleanup** session (§7.1), which acts solely on messages this feature processed, re-verifies each one by Message-ID hash and `UIDVALIDITY` first, and never issues a bare `EXPUNGE`. With `after_success = none` (default) the mock-server test asserts that no `STORE`, `COPY`, `MOVE`, `EXPUNGE` or `APPEND` is ever sent. |
| Data minimisation | Search server-side for Bandcamp-related messages only (`SINCE` date plus `BODY "bandcamp.com/download"`, with a client-side filter as fallback if the server rejects it). Skip messages larger than 1 MiB. Keep only: UID, UIDVALIDITY, SHA-256 of Message-ID, date, sender **domain**, and extracted links. Never store subjects, bodies or recipient addresses. |
| Hostile content | Never render HTML or load remote content (the sample email carries a 1×1 open-tracking image on `email.bandcamp.com`; it is never fetched, which would also tell the sender you opened the mail); no attachment saving; recurse into `message/rfc822` parts only to depth 2 (forwarded-as-attachment); cap parts and links per message (200 links). |
| Link safety | Only URLs passing the strict redeem validator (§6.2) are ever fetched: `https`, host exactly `bandcamp.com` or a subdomain of it, path `/download`, no userinfo, default port. Tracking redirects are **unwrapped by parsing**, never followed by fetching. |
| Logging | `tracing` events carry counts and categories only. The `sig` and other query parameters of links, passwords and message text are never logged; display URLs are redacted (`https://artist.bandcamp.com/download?…`). |
| Lockout and abuse | One connection attempt per button press, 15 s cool-down between checks, 60 s lockout after an authentication failure, and after 3 consecutive failures the stored password is flagged "needs re-entry". Overall deadline 120 s, per-command timeout 30 s, user-cancellable. |
| Stored links | Links are retained for retries, then purged 30 days after success or expiry (setting). "Forget history" deletes everything the feature stored. |

## 4. Dependencies (`bcvault-core/Cargo.toml`, behind feature `mail`, enabled by default)

```toml
async-imap   = { version = "0.11", default-features = false, features = ["runtime-tokio"] }  # default runtime is async-std: disable it
tokio-rustls = "0.26"            # same rustls family as reqwest's rustls-tls
webpki-roots = "1"               # verify the current major with cargo add
mail-parser  = "0.11"            # a 1.0 line exists but needs a far newer Rust toolchain; stay on 0.11 unless you upgrade
keyring      = { version = "3", default-features = false, features = ["apple-native", "windows-native", "sync-secret-service"] }
secrecy      = "0.10"
zeroize      = "1"
sha2         = "0.10"            # message-id and link hashing; verify the current version
```

`scraper`, `url`, `regex` (or hand-written scanners), `percent-encoding`, `chrono` are already present. Use `cargo add` to confirm current versions and make sure only one rustls and one tokio are in the graph (`cargo tree -d`). With `async-imap` on `runtime-tokio`, the stream type must implement tokio's `AsyncRead + AsyncWrite + Unpin + Send` (the TLS stream does); confirm the exact `Client::new` / `login` signatures against the pinned version.

## 5. Data model (`migrations/000N_mail.sql`, number by landing order)

```sql
CREATE TABLE mail_accounts (
    id INTEGER PRIMARY KEY, label TEXT NOT NULL,
    host TEXT NOT NULL, port INTEGER NOT NULL DEFAULT 993,
    security TEXT NOT NULL DEFAULT 'tls' CHECK (security IN ('tls')),     -- 'starttls' joins in stretch S1
    username TEXT NOT NULL, mailbox TEXT NOT NULL DEFAULT 'INBOX',
    my_address TEXT,                                -- shown/copied for the Bandcamp form
    uidvalidity INTEGER, last_uid INTEGER,          -- incremental cursor
    first_check_window_days INTEGER NOT NULL DEFAULT 30,
    download_mode TEXT NOT NULL DEFAULT 'auto' CHECK (download_mode IN ('auto','review')),
    after_success TEXT NOT NULL DEFAULT 'none'
        CHECK (after_success IN ('none','mark_read','move_folder','trash','delete')),
    after_success_folder TEXT, trash_folder TEXT,       -- trash_folder: \Trash special-use mailbox, else chosen by the user
    cleanup_include_already_have INTEGER NOT NULL DEFAULT 0 CHECK (cleanup_include_already_have IN (0,1)),
    only_from_bandcamp INTEGER NOT NULL DEFAULT 0 CHECK (only_from_bandcamp IN (0,1)),   -- soft filter on From domain
    last_checked_at INTEGER, auth_failures INTEGER NOT NULL DEFAULT 0, locked_until INTEGER
) STRICT;                                           -- NO password column, ever

CREATE TABLE mail_messages (
    account_id INTEGER NOT NULL REFERENCES mail_accounts(id) ON DELETE CASCADE,
    uidvalidity INTEGER NOT NULL, uid INTEGER NOT NULL,
    msgid_hash BLOB, received_at INTEGER, from_domain TEXT,
    link_count INTEGER NOT NULL DEFAULT 0, truncated INTEGER NOT NULL DEFAULT 0,
    cleanup_state TEXT NOT NULL DEFAULT 'none' CHECK (cleanup_state IN ('none','pending','done','failed','skipped')),
    cleanup_attempts INTEGER NOT NULL DEFAULT 0, cleaned_at INTEGER,
    PRIMARY KEY (account_id, uidvalidity, uid)
) STRICT;
CREATE INDEX idx_mailmsg_hash ON mail_messages(account_id, msgid_hash);

CREATE TABLE mail_links (
    link_hash BLOB PRIMARY KEY,                     -- SHA-256 of canonical URL
    account_id INTEGER NOT NULL, uidvalidity INTEGER NOT NULL, uid INTEGER NOT NULL,
    url TEXT NOT NULL,                              -- full link incl. signature: needed for retry; purged by retention
    item_type TEXT, item_id INTEGER, title_hint TEXT, artist_hint TEXT,   -- from the download page
    state TEXT NOT NULL CHECK (state IN ('new','resolved','queued','downloaded','already_have','expired','invalid','unmatched','failed','ignored')),
    first_seen_at INTEGER NOT NULL DEFAULT (unixepoch()), last_attempt_at INTEGER, error TEXT
) STRICT;
CREATE INDEX idx_maillinks_state ON mail_links(state);
CREATE INDEX idx_maillinks_item  ON mail_links(item_type, item_id);

CREATE TABLE mail_wait (                            -- "waiting for email" chips
    item_type TEXT NOT NULL, item_id INTEGER NOT NULL, requested_at INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (item_type, item_id)
) STRICT;
```

`Settings` gets nothing secret. Per-account options live in `mail_accounts`. If `LIBRARY_SYNC_PLAN.md` has landed, the redeem commit sets `origin = 'redeem'` and `local_status = 'present'`.

## 6. Check pipeline (`mail/`)

```text
mail/
├── mod.rs          MailService (worker task), MailSource trait
├── imap.rs         ImapSource: connect (tokio + rustls), login, examine, search, fetch
├── creds.rs        keyring wrapper, SecretString lifecycle, session-only fallback
├── parse.rs        mail-parser → ParsedMessage { received_at, from_domain, text, html, nested }
├── links.rs        extraction, unwrapping, validation, canonicalisation, dedupe hash
├── resolve.rs      download-page fetch via gate → items → ResolvedLink
├── matcher.rs      ResolvedLink → ItemKey (uses library::textmatch and library::matcher scoring, shared with the library plan)
└── errors.rs       typed MailError categories → user-facing messages
```

```rust
pub trait MailSource: Send {
    async fn open(&mut self) -> Result<MailboxInfo, MailError>;                       // uidvalidity, exists
    async fn search_new(&mut self, cursor: Cursor, cap: u32) -> Result<Vec<u32>, MailError>;
    async fn fetch(&mut self, uids: &[u32]) -> Result<Vec<RawMessage>, MailError>;     // BODY.PEEK, size-capped
    async fn close(self) -> Result<(), MailError>;
}
```

`ImapSource` is the real implementation; `FakeMailSource` serves `.eml` fixtures for pipeline tests.

**Stages**

1. **Pre-flight.** Check cool-down and lockout; load the secret; emit `MailProgress::Connecting`.
2. **Connect and log in.** TCP connect (10 s), TLS handshake, greeting, `LOGIN` (or `AUTHENTICATE PLAIN` if `LOGINDISABLED`), 30 s per command. Map failures to categories: DNS, refused/timeout, TLS certificate, authentication failed, mailbox not found, server busy, protocol error.
3. **Select read-only** (`EXAMINE mailbox`). If `UIDVALIDITY` differs from the stored one, discard the cursor and fall back to the first-check window (links are deduplicated by hash, so nothing is downloaded twice).
4. **Search.** First check or after a UIDVALIDITY change: `UID SEARCH SINCE <today − window> BODY "bandcamp.com/download"`. Later checks: `UID SEARCH UID <last_uid+1>:* BODY "bandcamp.com/download"`. If the server rejects `BODY`, fetch headers for the range and filter client-side. Cap at 500 messages per check (newest first) and report "N more available: Check again". User-selectable "Rescan the last N days" for recovery.
5. **Fetch** in batches of 25 with `BODY.PEEK[]`, skipping messages above 1 MiB (counted as skipped).
6. **Parse** with `mail-parser` (text and HTML bodies, nested `message/rfc822` parts to depth 2), all in plain synchronous functions that return owned structs.
7. **Extract and validate links** (§6.2); record each message in `mail_messages` and each new link in `mail_links` (state `new`).
8. **Resolve** each `new` link: `RedeemClient::fetch_download_page(url)` through `gate.acquire(Page)` (existing base-plan code, refactored to be callable here), parse `#pagedata[data-blob]` to `download_items`. Empty list becomes `expired` ("link expired or already used"); parse failure becomes `invalid`. Resolution is sequential and paced by the gate (about 1.5 s each); progress is reported per link.
9. **Match** each resolved item (§6.3) to a release and decide the state.
10. **Hand off.** Matched and not yet downloaded items become `RedeemResolved` jobs (auto) or appear in the review dialog. `mail_wait` rows for those releases are cleared when the job succeeds.
11. **Close** the read-only session. Update `last_uid`, `uidvalidity`, `last_checked_at`; reset `auth_failures`. Email cleanup, if enabled, happens later in its own short session once the downloads have finished (§7.1).

### 6.1 Fixture capture (task M1.1)

**Fixture 1 (received).** A real "Your download from …" email, as stored by Gmail. What it tells us:

| Observation | Consequence for the design |
| --- | --- |
| `From` display name is the **artist name** (RFC 2047-encoded, here made of unusual invisible-looking characters), address `noreply@bandcamp.com`; subject is `Your download from <artist>` | Artist hint = From display name. Names can be odd or blank-looking, so never use them as a hard key and never normalise them away to empty. |
| `Reply-To: noreply@<artist>.bandcamp.com` | The artist's subdomain, i.e. the base URL for **Add artist** on unmatched links, with no extra request. |
| Body (plain and HTML) says: `To start your free download of "<title>," please click the following link:` | Title hint, extracted best-effort with a tolerant pattern (the comma may sit inside the quotes); never required, because wording may change or be localised. |
| Link: `https://bandcamp.com/download?from=email&id=<n>&payment_id=<n>&sig=<32 hex>&type=album` | Host is plain `bandcamp.com` (not an artist subdomain), path `/download`, so the validator already accepts it. `type` and `id` give a **hint** of the item before any request (§6.3 stage 0). |
| The same link appears in `text/plain` and in the HTML `<a href>`, and the HTML href uses a raw `&` (not `&amp;`) | Extraction must accept both forms and deduplicate to one link. |
| HTML ends with `<img src="http://email.bandcamp.com/wf/open?upn=…">` | Open-tracking pixel: path is not `/download`, so the validator rejects it; the app must also never fetch it. Fixture negative. |
| Multipart layout is `multipart/alternative` with a nested `multipart/related` HTML part, 8-bit UTF-8 | Parser must flatten nested parts. |
| Gmail's own `Authentication-Results` show DKIM `email.bandcamp.com`, DMARC pass | Not used for logic (forwarded copies lose it); `From` domain `bandcamp.com` is only a soft signal and the basis of the optional *Only messages from bandcamp.com* filter. |

Expected manifest entry for it: one canonical link (query sorted), `type = album`, `id = 545785383`, title hint `shivering fox`, artist subdomain hint from `Reply-To`, `from_domain = bandcamp.com`, tracking pixel rejected, link count 1 after text/HTML deduplication.

**Sanitise before committing.** The raw sample contains a working signed link (`sig`, `payment_id`) and a real recipient address. The sanitiser must replace the recipient, `sig`, `payment_id` and the `upn` token while preserving their length and character class, so fixtures keep their shape but grant nothing. Do not commit raw `.eml` files.

**Still wanted:** one sample **as it arrives in the cock.li mailbox after your forwarding**, to confirm that forwarding keeps `From`, `Reply-To` and the body intact (an automatic forward usually does; "forward as attachment" or a manual forward wraps it differently, and the parser handles nested messages either way).
The owner supplies 6 to 10 `.eml` files exported from the mailbox (Thunderbird "Save as…" works), sanitised by a helper script `scripts/sanitize_eml.py`/`.rs`: replaces recipient addresses and any token that is not part of the `download` link, but **keeps link structure**. Needed cases: a plain Bandcamp download email, one forwarded inline, one forwarded as `message/rfc822` attachment (if you do that), an HTML-only one, a quoted-printable soft-wrapped one, one for a multi-item or bundle (if any), and two unrelated non-Bandcamp emails as negatives. A manifest records, per file, the expected links and sender domain. Assumptions to confirm from them: link host and path shape, query parameter names, whether links are wrapped by a click tracker, what the `From` domain looks like after forwarding.

### 6.2 Link extraction and validation (`links.rs`)
- Collect candidates from `text/plain` (URL scanner that handles angle brackets, trailing punctuation, `format=flowed` line joins) and from HTML (`<a href>`; accept both `&amp;` and raw `&` in query strings, as in the real sample). Only anchors and visible text are read; `<img>` sources are ignored entirely.
- **Hints** (never trusted, never required): from the URL query `type` and `id` (`LinkHints { item_type, item_id, from_email }`), and from the message `MessageHints { artist_display (From name), reply_to_subdomain, title_quote }`. Query parameters are kept verbatim for the canonical URL and dedupe; their meaning is only used for hints.
- **Unwrap trackers** only by parsing: if a URL has a query value that percent-decodes to a candidate that itself passes validation, use that.
- `validate_redeem_url(url) -> Result<CanonicalUrl, LinkReject>`, the same function as the manual **Redeem URL…** modal: scheme `https` (upgrade `http`), host `bandcamp.com` or ending in `.bandcamp.com` **as a whole label match** (reject `evilbandcamp.com`, `bandcamp.com.evil.net`, `bandcamp.com@evil.net`, punycode lookalikes), no userinfo, default port, path exactly `/download`. Everything else (album pages, unsubscribe links, images) is dropped silently.
- Canonical form: lowercase host, query parameters sorted, fragments removed. `link_hash = SHA-256(canonical)`; a link already in `mail_links` is skipped, whatever its state (so a link that was downloaded stays "already have").
- Forwarded copies of the same email produce the same links and are deduplicated by hash; the same Message-ID across UIDs is deduplicated by `msgid_hash`.

### 6.3 Matching (`matcher.rs`)
0. **Pre-check from the email (saves a request, optional).** If the URL hint `(type, id)` hits a `releases` row **and** the title hint normalises equal to that row's title **and** the row is already downloaded and present, mark the link **Already have** without fetching the download page. The `id` in the link is assumed to be the item id but may be a sale-item id; a hint alone never decides anything, so any mismatch or missing title simply falls through to the normal resolve. The hint also clears matching `mail_wait` chips for display.
1. After resolving, exact `(item_type, item_id)` from the download page against `releases` → **Matched**.
2. Else normalised `(artist, title)` through the library's scorer (`library::textmatch::key`/`tokens` and `library::matcher::score_candidate`; adapt or extract a shared helper rather than duplicating it). `score ≥ auto_match_min` and a single candidate → **Matched (probable)**; below it → **Review**.
3. Matched release already `downloaded = 1` (and `local_status` not `missing`) → **Already have** (not downloaded again unless the user ticks it).
4. No match → **Unmatched**: offer *Add artist* using the link host when it is an artist subdomain (or the artist URL in the download page if the fixtures show one); after the artist's first sync the app re-runs matching for that link.
5. Download page contains several items: each item is matched and listed separately.

A matched release keeps its classification `email_required` (as in base-plan §6.9) and is marked downloaded after success.

## 7. Download integration (`worker/downloads.rs`, small refactor)

- Extract the redeem page parsing and format choice from `redeem.rs` into a function returning `ResolvedRedeem { items, fetched_at }`. The existing `RedeemUrl` command keeps working by calling it.
- New command variant `RedeemResolved { target: ItemKey, resolved: Arc<ResolvedRedeem>, item_index: usize, opts: DownloadOpts, mail_link: Option<LinkHash> }`. Resolved pages are cached in memory for 10 minutes so the download does not fetch the page twice; after expiry the job refetches via the gate.
- Everything after that is the existing path: format choice and fallback, streaming, `.part` handling, extraction, atomic rename, one writer transaction, events. `download_history.source = 'redeem'`, `counts_against_quota = 0`.
- On success the `mail_links.state` becomes `downloaded`; on `EmptyDownloadItems` during the job it becomes `expired`; other failures become `failed` with the error category and are retryable from the dialog.

### 7.1 Email cleanup after successful downloads (`mail/cleanup.rs`)

Per-account setting `after_success`: **Leave the email** (default) · **Mark as read** · **Move to folder** · **Move to Trash** · **Delete permanently**. Trash is the recommended choice (recoverable); permanent deletion shows a one-time confirmation when enabled. Cleanup touches only the mailbox copy (for example your cock.li inbox), never the original in the account you forward from.

**Which messages qualify (conservative):**
- Every link found in the message is in a terminal state, and at least one is `downloaded` by this feature. The other links may be `downloaded`, or `already_have` only if *Also clean up emails for releases I already have* is ticked (default off).
- A message with any link that is `failed`, `expired`, `unmatched`, still `queued`/`resolved`, or `ignored` is left alone, as is any message without links or without a Message-ID. A message whose links only finish later (for example after *Add artist*) qualifies at that moment.

**When it runs.** Downloads outlive the check session, so cleanup is a separate step. When all jobs from a check's batch are terminal and the account has cleanup enabled, qualifying messages are marked `cleanup_state = 'pending'` (persisted) and a short cleanup session runs automatically, because you opted in. If the app was closed, the server was unreachable or the step failed, the messages stay `pending` and are processed at the start of your next **Check mail** press, or from a **Clean up now** button. There is no background retry, and each message gets at most 3 attempts.

**Cleanup session (the only read-write session):**
1. Connect and log in as usual, check `CAPABILITY` (`UIDPLUS`, `MOVE`, special-use), `SELECT` the mailbox.
2. Stored `UIDVALIDITY` must match; otherwise those messages become `skipped`.
3. For each pending UID, `UID FETCH uid (BODY.PEEK[HEADER.FIELDS (MESSAGE-ID)])` and compare its SHA-256 with `msgid_hash`. Mismatch means `skipped`, and nothing is done to that message.
4. Act:
   - *Mark as read*: `UID STORE uid +FLAGS.SILENT (\Seen)`.
   - *Move to folder / Trash*: `UID MOVE`; without `MOVE`, `UID COPY` followed by `\Deleted` plus `UID EXPUNGE` of that UID only (needs `UIDPLUS`). The Trash target is the mailbox with the `\Trash` special-use attribute, else the folder chosen in settings; with neither, nothing is moved and the app asks you to pick one.
   - *Delete permanently*: `UID STORE uid +FLAGS.SILENT (\Deleted)` then `UID EXPUNGE <set>`, **only** with `UIDPLUS`. Without it the app falls back to the Trash behaviour and says so. A bare `EXPUNGE` is never sent, because it would also purge unrelated messages you had flagged.
5. Record `cleanup_state = 'done'` and `cleaned_at`, or `failed` with the error category; summary toast, for example "Moved 3 emails to Trash".

In **Review first** mode the results list shows "Clean up N emails after downloading" as a checkbox, defaulting to the setting.

## 8. Worker API

```rust
// UiToWorkerCommand additions
SaveMailAccount { account: MailAccountInput },       // password only inside this message; handled once, then zeroized
TestMailAccount { account: MailAccountInput },
ForgetMailPassword { account_id: i64 }, RemoveMailAccount { account_id: i64 },
CheckMail { account_id: i64, mode: MailCheckMode, opts: DownloadOpts },   // New | RescanDays(u32) | Continue
EnqueueMailLinks { link_ids: Vec<LinkId>, opts: DownloadOpts },
IgnoreMailLink { link_id: LinkId }, RetryMailLink { link_id: LinkId },
NoteEmailRequested { key: ItemKey },                  // sets mail_wait when the user opens the browser
CancelMailCheck, ForgetMailHistory,
CleanupMail { account_id: i64 },                      // "Clean up now"; also invoked internally at the end of a batch
SetMailDownloadMode { account_id: i64, mode: DownloadMode },   // Auto | Review; CheckMail also accepts a one-off override
// WorkerToUiEvent additions
MailTestResult { ok: bool, detail: MailTestDetail },  // folders, message count, or categorised error
MailProgress { phase: MailPhase, done: u32, total: u32 },
MailResults { batch: MailBatch },                     // candidates grouped by status
MailFinished { summary: MailSummary },
MailCleanupResult { account_id: i64, done: u32, skipped: u32, failed: u32, action: AfterSuccess },
MailAccountState { account: MailAccountView },        // last check, lockout, "needs password"
```

`MailBatch.candidates: Vec<MailCandidate { link_id, display_url (redacted), received_at, from_domain, status, item_key, artist, title, formats_available }>` with `status ∈ Matched | Probable | AlreadyHave | Unmatched | Expired | Invalid | Failed`.

## 9. UI specification (classic; FX restyles it)

| Place | Content |
| --- | --- |
| Toolbar | **Check mail** split button (shows last-checked time, disabled during a check or cool-down), next to the *Email Required* filter chip. The dropdown arrow offers *After checking: Download automatically / Review first* (changes the setting) and *Rescan last N days…*. Counter badge of "waiting for email" releases. |
| `EmailRequired` row actions | `Open in Browser` (also records `mail_wait`), `Copy my email address`, `Redeem URL…` (unchanged), and when a link was already found: `Download from mail`. |
| **Mail settings dialog** (Settings → Mail) | Provider preset combo (cock.li first), host, port, security (TLS), username, password (masked, "Store in system keyring" checkbox, "Forget password"), mailbox name with a **Test connection** button that lists folders and the message count and shows a categorised error otherwise, "My address" (for copying), first-check window (days), **After checking** (Download automatically / Review first), **After a successful download** (Leave the email / Mark as read / Move to folder / Move to Trash / Delete permanently, with a one-time confirmation for permanent deletion), "Also clean up emails for releases I already have", "Only use messages from bandcamp.com", a **Clean up now** button, retention days, "Forget mail history". Plain-language note on what is read and stored. |
| **Results dialog** | Opens after a check in **Review first** mode, or in either mode when anything needs attention. Groups: **Ready** (checked), **Already downloaded** (unchecked), **Not in your library** (with *Add artist*), **Expired / invalid**, **Failed** (with *Retry*). Footer: `Download selected (n)`, `Close`. Counts animate in FX. |
| Dock | A `MAIL` chip while checking (phase and progress: connecting, searching, reading N/M, resolving N/M); cancel button; errors appear in the Errors tab with category and a one-line fix ("Authentication failed: check the password; locked for 60 s"). |
| First run | If no account exists, **Check mail** opens the settings dialog with the cock.li preset selected. |

FX extras: envelope-style slide-in of the results list, cover thumbnails next to matched items (from the existing thumbnail pipeline), palette commands "Check mail" and "Mail settings", drawer chip "link found in email on <date>".

## 10. Error categories (`errors.rs`)

| Category | Cause | User message and behaviour |
| --- | --- | --- |
| `Dns`, `Unreachable`, `Timeout` | network | "Could not reach <host>"; no retry loop |
| `TlsCertificate` | invalid or mismatched certificate | "Certificate not trusted"; never offer to ignore |
| `AuthFailed` | wrong credentials | start 60 s lockout, increment `auth_failures`, flag at 3 |
| `MailboxMissing` | folder not found | list folders and let the user pick |
| `ServerBusy`, `Protocol` | server-side | shown once with the raw (redacted) server text in the log |
| `TooManyMessages` | cap reached | partial results delivered, "Check again" offered |
| `ParseSkipped` | oversized or malformed message | counted, not fatal |

## 11. Politeness and limits

One read-only IMAP session per press (plus one short cleanup session when you enabled cleanup and that press's downloads have finished); no background activity; Bandcamp fetches go through the gate (page pacing, cooldowns); the 15 s check cool-down also protects the mail provider; downloads obey `max_concurrent_downloads` and the per-artist semaphore; redeem downloads do not touch the quota counters (as in the base plan).

## 12. Implementation phases and milestones

### Phase M0: Prerequisites (1 day)
reuse of `library::textmatch` and `library::matcher` scoring (shared with the library plan), refactor `redeem.rs` into `fetch_download_page` plus `ResolvedRedeem`, `MailSource` trait with `FakeMailSource`, feature flag `mail`, migration, repos.

**Milestone**
- [ ] Existing redeem tests (Phase 3) still pass unchanged.
- [ ] Migration applies; `mail_accounts` has no secret column (schema test).

### Phase M1: Parsing and link extraction (2 days)
M1.1 fixture capture and sanitiser (owner supplies emails); `parse.rs`; `links.rs`; shared `validate_redeem_url` used by both the modal and the mail flow.

**Milestone**
- [ ] Every fixture yields exactly the links pinned in its manifest; negatives yield none. Fixture 1 specifically: one link after text/HTML deduplication (raw `&` handled), the tracking pixel ignored, hints extracted (`type = album`, title hint, `Reply-To` subdomain, odd artist display name kept intact).
- [ ] Hostile-URL table all rejected: `evilbandcamp.com`, `bandcamp.com.evil.net`, `https://bandcamp.com@evil.net/download`, `http://` with a non-bandcamp host, punycode lookalike, `/downloads`, extra port, `javascript:`.
- [ ] Quoted-printable soft wraps, `&amp;` entities, `format=flowed` and nested `message/rfc822` handled; duplicate links in one message collapse to one.
- [ ] A 200-link or 1 MiB message is capped, not crashed on (fuzz-style test with random MIME).

### Phase M2: IMAP client, credentials, test connection (2 days)
**Milestone**
- [ ] Against a scripted mock IMAP server (tokio TCP, plain transport allowed only under `cfg(test)`): greeting, login, `EXAMINE`, `UID SEARCH`, `UID FETCH` all work; the server records commands and the test asserts none of `STORE/COPY/MOVE/EXPUNGE/APPEND` was sent with `after_success = none`.
- [ ] `UIDVALIDITY` change resets the cursor; a wrong password yields `AuthFailed` and the lockout, with exactly one login attempt per press.
- [ ] Password round-trips through the keyring in a manual test on each OS you use; with the keyring unavailable it falls back to session memory and writes nothing to disk.
- [ ] Captured `tracing` output of a full check (including a failed login) contains neither the password nor any link `sig`.
- [ ] Manual: **Test connection** against your cock.li account lists folders and the message count.

### Phase M3: Pipeline, resolve, match (2 days)
**Milestone**
- [ ] With `FakeMailSource` plus wiremock download pages: a 5-email mailbox produces the expected candidate groups (matched, probable, already have, unmatched, expired); the gate shows page requests spaced as configured and zero requests to non-Bandcamp hosts.
- [ ] A second check with no new mail issues zero Bandcamp requests; a rescan of the same window does not re-resolve links already in `mail_links`.
- [ ] Cursor and dedupe: forwarded duplicates and re-delivered UIDs produce one link each.

### Phase M4: Download integration and cleanup (2 days)
**Milestone**
- [ ] End to end (mock mail + wiremock Bandcamp): matched links download, the release row is marked downloaded with `redeem` history (`counts_against_quota = 0`), `mail_links` states become `downloaded`, `mail_wait` clears.
- [ ] An expired link ends as `expired` with no file and no stuck job; cancelling mid-check leaves no half-written state.
- [ ] Quota guard behaviour is unchanged for non-redeem downloads.
- [ ] Cleanup against the mock IMAP server: with `trash`, only the targeted UIDs are moved (`UID MOVE`, or `COPY` + `\Deleted` + `UID EXPUNGE <uid>` without `MOVE`); with `delete` and no `UIDPLUS` it falls back to Trash and never sends a bare `EXPUNGE`; unrelated messages (including ones pre-flagged `\Deleted`) are untouched.
- [ ] A message with one downloaded link and one expired or failed link is not cleaned; an `already_have`-only message is cleaned only when the toggle is on.
- [ ] A `UIDVALIDITY` change or a Message-ID hash mismatch aborts that message's cleanup (`skipped`) without any `STORE`/`MOVE` on it.
- [ ] Killing the app between download and cleanup leaves messages `pending`; the next **Check mail** press processes them first; no cleanup ever happens without a button press or the end of a press's own batch.
- [ ] Both download modes work end to end, including the one-off override from the dropdown.

### Phase M5: Classic UI (2 days)
**Milestone**
- [ ] First-run flow: **Check mail** opens the cock.li preset; Test connection, save, check works end to end against your real mailbox for one real email-gated release.
- [ ] Automatic and Review first both behave and the Check mail dropdown switches between them; unmatched items offer *Add artist* (using the `Reply-To` subdomain hint when present) and complete after its sync.
- [ ] Cleanup settings persist; enabling *Delete permanently* shows the one-time confirmation; **Clean up now** processes pending messages.
- [ ] UI thread stays responsive during a check (frame time unchanged, no blocking calls: the existing `block_on` grep guard prints nothing).

### Phase M6: FX integration and hardening (1 day)
FX restyling, palette commands, privacy text, `docs/mail.md` (what is stored, how to revoke, how to remove), retention purge job on startup, optional stretch items.

**Milestone**
- [ ] "Forget mail history" and "Remove account" leave no links, messages or keyring entry behind (test plus manual check).
- [ ] FX and classic show the same mail state from the shared DB; square-look guard still prints nothing.

### Stretch
- **S1 STARTTLS and certificate pinning** (Proton Bridge, self-signed servers).
- **S2 OAuth2** for Gmail/Outlook (needs registered client ids; likely not worth it).
- **S3 Auto-check on app start** (opt-in only; still no background polling).

## 13. Testing summary

`FakeMailSource` pipeline tests; scripted mock IMAP server for the protocol layer; wiremock for download pages and files; fixture-driven parsing; negative-URL table; secret-leak test (capture `tracing` output, assert absence of password and `sig` values); read-only assertion on the mock server; keyring behaviour behind a trait (`SecretStore` with a memory implementation for tests); optional `mail-live-tests` feature (manual, env-var credentials of a throwaway mailbox) to detect provider drift.

## 14. Risks and open questions

| Risk | Mitigation |
| --- | --- |
| Email layout and link format are unverified | Fixture-first development (§6.1); link detection is host/path based, not template based. |
| Links expire or are single-use | `expired` state, newest-first processing, clear message; nothing is retried automatically. |
| Forwarding changes sender and wraps content | No sender allowlist by default; links found anywhere in the message; nested forwards parsed. |
| cock.li availability or blocking | Categorised errors, short timeouts, Test connection, no retry storms. |
| Stored links are sensitive (they grant a download) | Local DB only, retention purge, "Forget history", redacted in UI and logs. |
| Keyring unavailable on some Linux setups | Session-only fallback; documented. |
| Accidental large downloads from a big mailbox | 500-message cap, 30-day first-check window, Review first mode, normal queue limits. |
| Cleanup removes an email you still needed | Off by default; Trash recommended; only fully successful messages qualify; per-message Message-ID and `UIDVALIDITY` verification; targeted `UID EXPUNGE` only; permanent delete needs a one-time confirmation. |
| The link's `id` may not be the item id | Used only as a hint with a title check; the download page's own item id decides (§6.3). |
| Dependency drift (`async-imap` 0.11.x, `mail-parser` 0.11 vs 1.0) | Pinned versions, thin `MailSource` boundary, parsing isolated in `parse.rs`. |

**Defaults assumed; confirm or override:**
1. Download mode defaults to **Automatic**; **Review first** is selectable in settings and from the Check mail dropdown.
2. The mailbox is never modified unless you enable *After a successful download* (default: Leave the email; Trash is the recommended cleanup choice).
3. First check looks back 30 days; later checks use the UID cursor.
4. One account in the UI (schema allows several).
5. No automation of the artist's email form (you paste your address yourself; the app only copies it).
