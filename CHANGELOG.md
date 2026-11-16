# Changelog

## Unreleased (v0.1.4)

### Added

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

- View mode is now a global preference (`filehub.view`); switching directories keeps the selected list/thumbnail/tile layout, with old per-path values as fallback.
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
