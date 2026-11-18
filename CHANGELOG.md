# Changelog

## v0.1.8 - 2026-11-18

### Added

- `GET /api/v1/fs/image-info` header-only preview check; over-limit images show the dimensions and a download link without opening the lightbox.
- `thumbnails.preview_edge` (512–4096, default 2048) with an admin setting; preview thumbnails now use the long edge instead of the short edge.
- Admin settings for `thumbnails.jpeg_max_pixels`, `max_pixels`, and `max_source_bytes`, including source display and override reset.
- Structured job failure notifications with job kind, object label, and stable error code.
- Completion notifications for long copy/move/delete/upload operations, with thresholds and deduplication.
- Share visit/download/upload notifications for the creator, throttled per share/item with a local preference toggle.
- Setup accepts an optional `language` and stores it as the first admin's preference.
- `read_only_reason` on directory listings (`policy` / `filesystem`) and `writable` / `read_only_reason` / `line_ending` on `GET /file`.
- `FileName` component: single-line names with the extension always visible, used by delete, transfer, share, properties, and preview dialogs.

### Changed

- The preview blur layer stays mounted for the whole lightbox lifetime and fades to `visibility: hidden`; it no longer gets removed mid-fade.
- The blur layer uses a pre-blurred canvas with edge-pixel replication at the same size as the sharp image, removing the previous ~15% zoom and feathered edges. The sharp image is explicitly stacked above the persistent blur node so a compositor reorder cannot flash a blur-only frame.
- Animated GIFs use the original file as the sharp preview, so they play immediately; the preview/original toggle is hidden for GIFs and SVGs.
- Small images are capped at their natural width at 100% zoom; large images still fit the viewport.
- Double-clicking an SVG opens the image lightbox; "Edit source" is available from the lightbox and context menu. SVG payloads are rendered through a blob URL and are never served inline.
- Filesystem read-only directories are detected with `W_OK|X_OK` plus read-only mount checks; writes return `403 PERMISSION_DENIED`, while policy read-only keeps `403 FORBIDDEN`.
- Read-only directories and files open the editor in read-only mode, with save hidden, `Ctrl/Cmd+S` inert, and no editing-presence report.
- Copy/move target pickers disable confirmation when the destination is not writable.
- The editor confirms before closing unsaved changes, preserves CRLF on save, uses `wrap="off"` so line numbers stay aligned, and reports `too_large`/`not_utf8`/`binary` separately.
- Sorting uses per-column default directions, refreshes in place without unmounting the list, and highlights the active column; grid/tile views gained a sort menu.
- Live and indexed sorting now agree: directories always come first and the name secondary key follows the primary direction.
- The multi-select context menu only shows batch-capable actions: preview selected media, download as ZIP, share, copy, move, delete. Rename/play/properties are hidden for multi-select.
- View icons are now list, thumbnail (four tiles), and tile (two stacked tiles).
- Long names in the sidebar and transfer dialog use single-line ellipsis; the breadcrumb scrolls to the current directory and keeps the read-only tag visible.
- Thumbnail and background scan failures no longer produce user notifications; file-list 404s name the missing directory.

### Fixed

- `download?inline=true` returned `Content-Disposition: inline` for SVG because the subtype check compared against `svg+xml`; SVG is now always an attachment.
- JPEG pixel limit raised to 1 GP; images above 100 MP skip the 320px grid thumbnail and stale `too_large` failure records are invalidated when limits change.
- `PermissionDenied` IO errors were mapped to 500 and now map to 403 `PERMISSION_DENIED` across list, download, editor, and upload paths.
- Preview thumbnails no longer generate a 6–7.5MP image for 16:9 or 3:2 photos by default.
- Sort direction reset on column changes and stale-name ordering inconsistencies.
- Editor long lines, CRLF rewrites, silent close, and generic “cannot preview” messages.
- Delete confirmations wrapped long filenames and pushed the extension onto a second line.

### Blocking and responsiveness fixes

- Deleting, renaming, copying, moving and index cleanup no longer delete cache subtrees with a single case-insensitive `LIKE` transaction. Exact paths are invalidated in the request; descendants are removed in background batches using primary-key range predicates.
- `indexed_dirs` entries are removed through bounded `index_entries` batches, so FTS triggers no longer hold the SQLite writer lock for a whole subtree.
- Usage is no longer recomputed by a full home-directory walk for every upload session; uploads use the per-user cached snapshot, usage invalidation keeps the last value for a background refresh, the TTL is 15 minutes, and refreshes are serialized.
- Listing probes (`dir_mtime_ns` / `has_symlinks`) run in `spawn_blocking` instead of a Tokio worker thread.
- The admin page renders after loading users/paths/settings; usage, roots and status load in a second wave through one bulk roots endpoint.
- Settings are written through a single coalescing queue with optimistic UI and inline save state; single-value controls auto-save, while custom CSS and file uploads keep explicit buttons.
- `dir_size_use_logical` cache clearing and thumbnail failure-record cleanup run in background batches.
- Thumbnail cache pruning is rate-limited to once per minute instead of walking the whole cache after every generated image.
- Interactive copy/move jobs keep a reserved share of the job queue, and audit enqueueing is best-effort instead of blocking request handlers.
- Search runs on Enter or the search button, requires at least 3 characters, and uses one unified FTS query across the top-level roots. Covered child roots are skipped, so a root with few matches no longer forces a separate full FTS scan. Slow searches are logged. ZIP and bulk delete resolve all paths against one cached roots/links snapshot. API-token last-used updates are asynchronous and throttled.
- Search results use cursor pagination (100 per page, load more); the cursor follows the FTS rowid without a sorting pass, so common terms stay fast. Result keys include the root id, fixing duplicate-path render errors that left the UI stuck on "Searching…". Searches no longer trigger recursive full-index jobs.
- The audit page is rendered alone instead of underneath the still-mounted admin page; delete audit records now include the deleted path and item count.
- Switching storage roots no longer keeps the previous root's rows visible with "Calculating…" sizes; the view shows a loading state until the new root arrives.
- Admin status caches directory-size counts and bounds informational DB queries, so entering system settings no longer waits for a multi-second scan. Thumbnail jobs stay in memory instead of writing three SQLite rows each, and maintenance pruning is batched.
- Leaving a directory cancels queued list-thumbnail jobs for the old view; closing the lightbox cancels its preview job. Running short list thumbnails are allowed to finish, while long copy/move/delete/upload/index jobs are never cancelled by navigation.
- Settings focus/blur without a value change no longer writes to the database; grouped numeric fields only submit changed keys.
- The external-path alias input is focused and selected on the first click, so clicking elsewhere blurs and exits immediately.
- API token `last_used_at` writes are throttled to once every five minutes, and directory-size requests in the file browser are limited to six concurrent probes.

### Verification

- `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-targets`.
- `pnpm lint`, `pnpm check`, `pnpm test`, `pnpm build`.
- Non-root filesystem permission tests also run as `nobody`.


## v0.1.7 - 2026-11-17

### Added

- SSE event bus and authenticated `/api/v1/events` stream with heartbeat, per-user filtering, `X-Accel-Buffering: no` and scoped public-share events.
- Real-time job progress, usage updates, share changes, admin audit/slow-request events and notifications.
- Asynchronous thumbnails by default: cache misses return a transparent placeholder immediately and reload when `thumbnail.ready` arrives; `wait=1` keeps the synchronous 120-second path as a fallback.
- Editor presence hints (`edit.presence`) with a 60-second TTL and a "another tab is editing" indicator; saves are never blocked.
- Topbar notification center with unread count, local persistence and a small dropdown list.
- Token-scoped public share event stream for visits, downloads, uploads and thumbnails, with live feedback on the share page.
- Frontend EventSource client with BroadcastChannel forwarding, localStorage leader election and per-tab fallback.
- Optional folder upload path creation now creates each directory level and deduplicates within a batch.

### Changed

- Share management refreshes immediately on share events in addition to the 15-second fallback.
- Public share pages reconnect their event stream and fall back to synchronous thumbnails if the stream fails.
- Audio playback pauses automatically when another tab starts playing.
- Uploads run three files concurrently with a merged, per-file progress queue. Files can be paused, resumed, cancelled and retried individually; completed files leave the list immediately.
- Paused or interrupted uploads resume from the server-side chunk bitmap after the same file is selected again; active copy/move jobs are reattached after a page reload.
- Copy/move progress bars use bytes instead of file counts; interactive transfers use a separate worker pool so index and directory-size scans cannot leave a move at "queued".
- Image preview shows a blurred cached thumbnail clipped to the sharp image bounds, fades the sharp image in over 80ms, then removes the blurred layer; no feathered edges remain. Thumbnail long edge is capped at 4096.
- Full-size preview thumbnails use a reserved worker lane (`thumbnails.preview_workers`, default 1), so scrolling a folder full of large photos no longer blocks preview generation.
- Double-clicking an audio file opens the player instead of the lightbox; the player gained a compact persisted volume slider next to the controls, and opening a video pauses music until the video is closed. Video volume and mute state are persisted separately.
- The audio player is hidden while the lightbox is open; playback continues for images and is paused/resumed by video as before.
- Each directory pins its first resolved view and sort mode, so changing the view in a subdirectory no longer changes the parent directory.
- The preview blur uses the same 320px thumbnail as the grid, avoiding a duplicate thumbnail generation.
- Background index writes are committed in bounded chunks, the DB pool prefers idle connections, and cache invalidation for a path chain is applied in one transaction; slow deletes are logged with phase timings.
- Index invalidation now keeps the last directory-size value as `stale` instead of deleting it, and quick scans aggregate ready child sizes into the parent. Directories no longer fall back to "calculating" just because a quick scan marked their index partial.
- Directory-size scans have their own worker lane (`jobs.dir_size_workers`, default 1), so a long index or full scan cannot leave a directory at "calculating".
- Delete/copy/move completion only refreshes the list when the user is still viewing the source or destination directory; navigating away during a slow operation no longer pulls the view back.

### Fixed

- The terminal preference now actually hides the terminal entry and shortcut.
- Folder uploads accept zero-byte files and no longer fail the batch because of them.
- Windows filenames are validated by UTF-16 length, so long Chinese photo names no longer fail upload.
- Notification bell uses the same borderless icon-button styling as the theme toggle.
- Clicking a search result scrolls the matching entry into view.
- Upload preemption no longer raises a notice titled "Operation failed"; the paused file is shown in the queue instead.
- Info notices now default to a "Notice" title instead of "Operation failed".

### Verification

- `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --all-targets`.
- `pnpm lint`, `pnpm check`, `pnpm test`, `pnpm build`.

## v0.1.6 - 2026-11-17

### Added

- Video thumbnails seek to one third of the duration using ffmpeg metadata only; no ffprobe dependency. Probe failures fall back to the configured second offset.
- Per-directory view and sort preferences, with global defaults for directories that have no saved choice yet.
- Admin-controlled global custom CSS, served from a public immutable endpoint and applied to the app, login and share pages.
- Expanded file type icons: Windows executables/libraries/installers/shortcuts, fonts, disk images, ebooks, design files, databases, Office/WPS/iWork/OFD and more media formats.
- Drag-and-drop feedback: source dimming, valid/invalid target highlighting and move completion toast.
- Audio player uses play/pause/stop/previous/next icons; stop resets playback to 0:00.

### Changed

- System settings are grouped into feature switches, downloads/quotas, and new-user defaults. Disabling thumbnails, editor or terminal now hides the matching user preference row.
- New users inherit the current system defaults for hidden files, theme (including follow-system), language, upload conflict policy, quota and feature switches.
- Editor saves update usage by file-size delta instead of invalidating and rescanning the whole home directory.
- Deleting a user is now a soft delete: the row is retained, the user disappears from the admin list, sessions/API tokens are removed, shares are revoked, and the home directory is preserved.
- Soft-deleted and disabled users cannot authenticate; last-admin and self-delete checks remain.
- Stopped shares return 410 `SHARE_REVOKED`; expired shares return 410 `SHARE_EXPIRED`; unknown links remain 404. The share page shows the matching message.
- Admin refreshes keep the page mounted and preserve scroll position; external path and root removals require confirmation.
- Removed two duplicate indexes from `index_entries` (migration `0021`); new installs and upgrades stop maintaining redundant B-trees.

### Fixed

- Read-only context menus hide rename, move, delete, new file/folder and upload actions instead of leaving disabled entries.
- Audio player close button no longer loses clicks to the drag handler; long titles truncate without pushing the close button out.
- Admin user deletion requires typing the username before confirming.
- Admin pages no longer unmount the whole panel during background refreshes.

### Verification

- `cargo test --all-targets`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo fmt --check`.
- `pnpm lint`, `pnpm check`, `pnpm test`, `vite build`.
- Playwright coverage for admin grouping/preferences hiding, scroll preservation, soft delete, revoked shares, read-only menus, drag/drop, audio controls and file icons.

## v0.1.5 - 2026-11-17

### Fixed

- Context submenus for sort and view now open outside the first-level menu instead of being laid out inside its scroll container, which produced scrollbars and clipped or misplaced submenus.

### Changed

- Release publishing now builds bilingual release notes from `CHANGELOG.md` and `CHANGELOG.zh-CN.md`.

### Verification

- Playwright covers wide and narrow submenu placement, confirms the blank-area menu is no longer a scroll container, and checks that the file menu still scrolls.

## v0.1.4 - 2026-11-17

### Added

- Directory sizes use stale-while-revalidate: the last known value is returned immediately and shown dimmed while a background refresh runs; failed refreshes keep the previous value.
- A high-latency hint appears when startup or directory loading takes unusually long.
- Persisted quick-scan overflow candidates (`pending_index_dirs`, migration `0016`) so a burst of new directories is indexed across subsequent scans instead of being dropped after 500 entries.
- Read-only state for the current directory in list responses and public directory shares; the web UI greys out upload/write actions and shows a read-only breadcrumb label.
- Public share grid/list with thumbnails, selection, download, double-click preview, ZIP download and chunked uploads with cancel/retry.
- Public share media streaming with Range/ETag/inline responses, directory pagination and per-share rate limiting.
- Collection shares: one token for multiple selected files (new `share_items` table), excluding directories.
- Global branding: `ui.brand_name` plus a re-encoded PNG icon served from a public endpoint with ETag; login page, app shell, share page, title and favicon follow it.
- File properties dialog with Unix mode, symlink state, image dimensions and JPEG/TIFF EXIF metadata.
- Floating, draggable audio player for the main file browser and public share pages.
- Thumbnail diagnostics: failure reasons, `/thumb/status`, `/thumb/retry`, admin limits for source size/pixels/JPEG pixels.
- `log.retention_days` for Windows service log cleanup and `security.public_rate_limit_per_minute`.
- `--patch` / `--minor` / `--major` modes for `scripts/bump-version.mjs`, plus matching Makefile targets.
- Caddy and Nginx reverse proxy examples (forwarded headers, `trusted_proxies`, WebSocket, upload limits, buffering, timeouts) in both READMEs and `docs/operations.md`.
- systemd hardening recommendations and Windows service log retention docs.

### Changed

- View mode is now a global preference (`filehub.view`); switching directories keeps the selected list/thumbnail/tile layout, with old per-path values as fallback. The saved mode is applied before the first directory request instead of after it returns.
- Symlink state is shown as a chain icon before the file name (broken chain for broken or unreadable targets) instead of a badge on the file icon.
- The first screen falls back to the browser language instead of always English; signed-in user preferences still win.
- Share management refreshes on focus/visibility and every 15 seconds without reloading the page.
- Admin root changes (assign/remove/permission/external path) refresh the sidebar immediately without resetting the current browse position.
- Expired sessions, upload sessions and their `.filehub-upload-*.part` temp files are pruned hourly; stale `.filehub-edit-*.tmp` files are cleaned once per day with a bounded walk.
- File lists always show relative time (weeks/months/years included); the tooltip carries the absolute timestamp.
- `sort=size` uses indexed directory sizes and refreshes unknown sizes in the background.
- Thumbnail tile view now shares the 320px cache with the grid; JPEGs use DCT-scaled decoding (512 MiB / 250 MP defaults).
- Public share visits are counted once per root page load; thumbnails/downloads no longer inflate the counter.
- Audit IP uses the TCP peer unless it matches `security.trusted_proxies` (IP or CIDR), then honors forwarded headers.
- WAL tuning: explicit autocheckpoint, 64 MiB journal size limit, and TRUNCATE checkpoints on maintenance and graceful shutdown.

### Fixed

- Large JPEGs no longer return 404 because of the 50 MB/50 MP limits; damaged images report a reason instead of a broken tile.
- The quota card no longer stays at “统计中…” on first load; usage is polled until the background calculation finishes.
- Public directory shares no longer expose dotfiles or accept hidden paths for download/thumbnail/ZIP.
- Read-only directory shares no longer show an upload button.
- Short videos whose configured seek offset has no frame now fall back to the first frame instead of returning 404.
- Switching `jobs.dir_size_use_logical` clears cached directory sizes so both modes cannot mix stale values.
- Quick scans now queue up to 500 new directories per pass (was 100).
- Context menus are measured after rendering and clamped to the viewport, with keyboard navigation and scrollable submenus.
- Dialogs now trap focus and restore it on close.

### Verification

- `cargo test --all-targets`, including maintenance, read-only share/list, search-kind, quick-scan overflow, video-thumbnail, audit-IP, public-share, collection and properties tests.
- `cargo clippy --all-targets --all-features -- -D warnings` and `cargo xwin check --target x86_64-pc-windows-msvc`.
- `pnpm lint`, `pnpm check`, `pnpm test`, `vite build`.
- Playwright QA at 390/1024/1440 for language fallback, view persistence, read-only UI, share refresh, sidebar refresh, long filenames, virtualization, marquee, share grid/selection/preview/upload, collection shares, branding, properties and keyboard focus flows.

## v0.1.3 - 2026-11-16

### Added

- Persistent background job framework for directory sizes, copy/move, indexing, and thumbnails, with deduplication, bounded queues, progress, cancellation, and restart recovery.
- Background directory-size worker with nested-directory result caching, TTL refresh, active-ancestor deduplication, and indexed subtree-size aggregation.
- Scheduled index maintenance: quick scans every 10 minutes, periodic full scans, automatic index on first browse, and bounded recursive indexing for new subdirectories.
- Windows USN Journal incremental indexing with per-volume journal state, parent file-id path resolution, rename/delete subtree handling, and automatic MFT fallback.
- Index metadata for MIME type, creation time, preview availability, and hidden state, exposed through list and search APIs.
- Search filters for size range, modified time, file/directory type, and media kind, with a new filter panel in the web UI.
- Usage cache with background refresh, plus a batch usage endpoint for the admin page.
- Audit log keyset pagination, retention policy, and asynchronous batched writes with graceful-shutdown flush.
- Thumbnail negative-cache markers, small/large decode semaphores, configurable FFmpeg seek/timeout with process kill, true LRU eviction, and `If-None-Match`/304 support.
- Optional `du`-style directory sizes with Unix hardlink deduplication.
- Frontend request abort/generation guards, virtualized list and thumb/tile grids, transfer job progress/cancel, and page keep-alive for admin/settings/shares.
- Admin status page with job queues, directory-size cache, database/WAL sizes, index schedule, and recent slow requests.
- Migrations `0011` through `0015`.

### Changed

- Copy/move APIs now enqueue background transfer jobs instead of blocking the request.
- `POST /api/v1/index` now returns a job id and runs asynchronously.
- `/api/v1/usage` returns immediately with `computing=true` while a background refresh runs.
- Directory-size responses now include `queued`/`running`/`ready`/`error`, `partial`, `stale`, and `error` fields.
- Thumbnail generation is executed through the job manager, while the HTTP request waits for the shared result.
- Search index rows now carry richer metadata; old rows are marked stale once and rebuilt by the first quick scan.

### Fixed

- Large directory scans no longer block HTTP requests or browsing.
- Entering nested directories no longer triggers redundant parent/child scans.
- Repeated thumbnail failures no longer re-decode the same file on every request.
- Long searches no longer overwrite newer results in the web UI.
- Mobile layout overflow in the top bar and admin header.

### Upgrade notes

- Migrations run automatically on startup. Existing indexed directories are marked stale and rebuilt by the first quick scan (default within 10 minutes), or immediately via a manual index.
- New settings include `index.scheduled_scans`, `index.quick_scan_minutes`, `index.full_scan_hours`, and `jobs.dir_size_use_logical`.
- The Windows USN implementation is covered by cross-compilation checks but should be validated on the target NTFS volume before production rollout.

### Verification

- `make check`
- `cargo test --all-targets`
- `cargo xwin check --target x86_64-pc-windows-msvc`
- Playwright QA at 1440px and 390px, including 503-file list/thumb/tile virtualization.
