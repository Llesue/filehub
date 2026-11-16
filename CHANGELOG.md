# Changelog

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
