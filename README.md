# FileHub

[简体中文](README.zh-CN.md) | English

FileHub is a self-hosted file manager built around a single executable. It gives you a browser-based file browser, resumable uploads, sharing, search, thumbnails, and a small administration console without Docker, Node, or a separate database server at runtime.

The web UI is a Svelte 5 SPA embedded into the Rust binary. Data lives in SQLite (compiled in with `rusqlite`) and the filesystem. A release build is one static Linux binary; Windows builds use the same codebase and produce a separate `filehub.exe`.

![FileHub main interface](docs/assets/filehub-main-v2.png)

## Android app

A native Android client is available in a separate repository: [download the latest APK](https://github.com/Llesue/filehub-android/releases/latest).

## Why FileHub

Many years ago, on a WD My Cloud, I used FileBrowser for the first time and was amazed by its clean, elegant interface and how quickly it responded on low-powered hardware. Later, I needed multi-user authentication and the ability to manage arbitrary directories on my NAS, and FileBrowser gradually fell short of those needs. Then I was surprised to learn that FileBrowser had announced it was no longer being maintained—and the idea was born: build my own FileBrowser with the help of OpenCode and DeepSeek.

## Demo

https://filehub.ihtom.com

- Account: `demo` / `filehub2026`
- Public demo; the data resets every 24 hours
- Do not upload private or sensitive data; treat all uploads as untrusted

## Use cases

- Browser entry point for a home NAS or an external drive
- Moving files between phone and computer with resumable uploads
- Family photo/video library with password- and expiry-protected sharing
- Read-only archive for old disks that should never be modified
- Turning a Windows machine into a temporary file server
- Small-team file cabinet with multiple users, quotas, and external paths
- Editing server configs, logs, and docs directly in the browser
- Collecting event photos through a writable public share
- Running on low-power ARM devices or a single static Linux binary
- API clients and automation built on the documented HTTP API

## Features

- Browse, upload, download, rename, copy, move, and delete files and directories
- Chunked, resumable uploads, streamed multi-file ZIP downloads, and in-place ZIP archive/extract jobs
- Directory index with full-text filename search and sortable, multi-select search results
- Thumbnails for common image formats, with optional `ffmpeg` video frames
- Public file and directory shares with password, expiry, read-only mode, visit counts, and optional uploads for writable directory shares
- Inline text editor with ETag-based optimistic locking and Markdown preview
- Per-user preferences, avatars, quotas, and read-only/writable storage roots
- Admin console for users, external paths, quotas, per-user feature switches, expanded system settings, and audit logs
- Restricted command panel over WebSocket: `help`, `ls`, `tree`, `cp`, `mv`, `rm`, `zip`, `unzip`, `job`, `roots`, and safe-delete confirmation codes
- Federated remote directories: mount a directory exposed by another FileHub instance, read-only or read-write, with scoped tokens, invites, per-user switches, and proxied search, thumbnails, audio and ZIP
- Linux single-binary deployment; Windows LocalSystem service support

## Quick start

### Linux

The recommended way is to download a static release binary from [GitHub Releases](https://github.com/Llesue/filehub/releases). Choose the `x86_64` or `aarch64` musl build, verify it with the matching `.sha256`, then install it:

```bash
chmod +x filehub-*-x86_64-unknown-linux-musl
sudo install -m 0755 filehub-*-x86_64-unknown-linux-musl /usr/local/bin/filehub
```

To build from source instead:

```bash
make setup
make build
sudo install -m 0755 target/x86_64-unknown-linux-musl/release/filehub /usr/local/bin/filehub
```

Create the service user, configuration, and data directories. The config stays in `/etc/filehub`, while FileHub state lives under `/home/filehub`:

```bash
sudo useradd --system --create-home --home-dir /home/filehub --shell /usr/sbin/nologin filehub
sudo mkdir -p /etc/filehub /home/filehub/users /home/filehub/cache
sudo cp config.example.json /etc/filehub/config.json
sudo chown -R filehub:filehub /home/filehub
```

Missing data directories are created automatically at startup (`/home/filehub`, `users`, `cache`); make sure the service user owns them in advance.

Run in the foreground:

```bash
filehub --config /etc/filehub/config.json
```

Or use systemd:

```ini
[Unit]
Description=FileHub
After=network.target

[Service]
User=filehub
Group=filehub
WorkingDirectory=/home/filehub
ExecStart=/usr/local/bin/filehub --config /etc/filehub/config.json
Restart=on-failure
RestartSec=3
NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=strict
ReadWritePaths=/home/filehub
UMask=0077
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX
CapabilityBoundingSet=

[Install]
WantedBy=multi-user.target
```

`NoNewPrivileges` and an empty `CapabilityBoundingSet` prevent privilege and capability escalation, `PrivateTmp` isolates `/tmp`, `ProtectSystem=strict` makes the filesystem read-only, `ReadWritePaths` restores write access only for `/home/filehub`, `UMask=0077` keeps new files private, and `RestrictAddressFamilies` limits the service to IP and Unix sockets. If you move `users_root`, `cache_dir`, or `db_path` outside `/home/filehub`, add those paths to `ReadWritePaths` (use the parent directory for `db_path`). Do not set `ProtectHome=yes`: it hides `/home` and makes `/home/filehub` inaccessible to the service.

The default address is `http://<host>:8080/`. On first start, FileHub prints a setup token to the log (or uses `FILEHUB_SETUP_TOKEN`). Open the UI and use that token to create the first administrator.

### Windows

Download the Windows executable from [GitHub Releases](https://github.com/Llesue/filehub/releases) (`filehub-<version>-x86_64-pc-windows-msvc.exe`), verify it with the matching `.sha256`, and copy it to `C:\Program Files\FileHub\filehub.exe`.

To cross-compile from Linux instead:

```bash
cargo install cargo-xwin --locked
make build-windows
```

This produces `target/x86_64-pc-windows-msvc/release/filehub.exe`. Copy it to the Windows machine and, in an elevated PowerShell, install the service:

```powershell
& "C:\Program Files\FileHub\filehub.exe" service install
sc.exe start FileHub
```

The service runs as `LocalSystem`, starts automatically, and stores its database, users, cache, and logs under `C:\ProgramData\FileHub`. Service logs rotate daily to `%ProgramData%\FileHub\filehub.log.<date>` (for example `filehub.log.2026-11-16`). `log.retention_days` controls cleanup: the default is `14`, `0` disables cleanup, and the maximum is `3650`. Expired files are removed at startup and during the hourly maintenance pass.

To read the service log and retrieve the first-run setup token:

```powershell
Get-ChildItem C:\ProgramData\FileHub\filehub.log.* |
  Sort-Object LastWriteTime |
  Select-Object -Last 1 |
  Get-Content -Tail 50
```

Stop or uninstall:

```powershell
sc.exe stop FileHub
& "C:\Program Files\FileHub\filehub.exe" service uninstall
```

Windows indexing uses the NTFS MFT fast path when available, then USN Journal incremental updates; it falls back to a normal directory scan when USN is unavailable.

## Configuration

`config.json` supplies deployment defaults; values changed in the admin UI are stored in SQLite and take precedence.

The most important fields are:

```json
{
  "listen": "0.0.0.0:8080",
  "db_path": "/home/filehub/filehub.db",
  "users_root": "/home/filehub/users",
  "cache_dir": "/home/filehub/cache"
}
```

See [`config.example.json`](config.example.json) for all options. On Windows the defaults are under `C:\ProgramData\FileHub`; an explicit config path can be passed at installation time:

```powershell
& "C:\Program Files\FileHub\filehub.exe" --config "C:\ProgramData\FileHub\config.json" service install
```

Do not expose FileHub directly to the public internet. Put it behind a TLS reverse proxy, enable `security.cookie_secure`, and keep the data directory private.

## Reverse proxy

Do not expose FileHub directly to the public internet. Terminate TLS in Caddy or Nginx and forward traffic to FileHub over loopback or a private network. When the proxy runs on the same host, set `listen` to `127.0.0.1:8080` so FileHub cannot be reached around the proxy.

The proxy must preserve the original `Host` header and forward the client address in `X-Forwarded-For` and `X-Real-IP`, plus the public scheme in `X-Forwarded-Proto`. Overwrite these headers instead of appending to values supplied by the client; otherwise a client can forge its own audit IP.

FileHub only honors forwarding headers when the TCP peer is listed in `security.trusted_proxies`. Add the source address the proxy uses to connect to FileHub (`127.0.0.1`, `::1`, or the Docker bridge address/subnet). If the list does not match, audit records show the proxy address instead of the real client. Set `security.cookie_secure` to `true` whenever the public URL uses HTTPS; if you enable it while testing over plain HTTP, the browser will not store the session cookie.

```json
{
  "security": {
    "cookie_secure": true,
    "trusted_proxies": ["127.0.0.1", "::1"]
  }
}
```

Proxy requirements:

- WebSocket: `/api/v1/terminal/ws` needs HTTP/1.1 and the `Upgrade`/`Connection` headers. Caddy handles this automatically; Nginx needs the `map` block shown below.
- Upload body limit: set it above `uploads.chunk_size` (8 MiB by default). The examples use 32 MiB; raise it if you increase the chunk size.
- Downloads and ZIP streams: disable proxy buffering (`proxy_buffering off` and `proxy_request_buffering off` in Nginx, `flush_interval -1` in Caddy). Downloads do not send `X-Accel-Buffering`; realtime event streams (`/api/v1/events` and public share events) do send `X-Accel-Buffering: no`, but disabling buffering in the proxy is still recommended.
- Long transfers: the examples use a 1-hour read/send timeout. Increase it, or lower `downloads.zip_max_bytes`, if very large ZIP archives can take longer.

### Caddy

```caddyfile
files.example.com {
    request_body {
        max_size 32MB
    }

    # WebSocket upgrades (including /api/v1/terminal/ws) are proxied automatically.
    reverse_proxy 127.0.0.1:8080 {
        header_up Host {host}
        header_up X-Forwarded-For {remote_host}
        header_up X-Real-IP {remote_host}
        header_up X-Forwarded-Proto {scheme}

        flush_interval -1
        transport http {
            read_timeout 1h
            write_timeout 1h
        }
    }
}
```

Caddy obtains TLS certificates automatically and supports WebSocket upgrades through `reverse_proxy` without extra directives. The `header_up` lines replace forwarding headers that a client may have sent.

### Nginx

Define the upgrade map in the `http` context (for example in `/etc/nginx/conf.d/filehub-upgrade.conf`), then use a `server` block like this:

```nginx
map $http_upgrade $connection_upgrade {
    default upgrade;
    ''      close;
}

server {
    listen 443 ssl;
    server_name files.example.com;

    ssl_certificate     /etc/letsencrypt/live/files.example.com/fullchain.pem;
    ssl_certificate_key /etc/letsencrypt/live/files.example.com/privkey.pem;

    client_max_body_size 32m;

    location / {
        proxy_pass http://127.0.0.1:8080;

        # WebSocket: /api/v1/terminal/ws uses the upgrade map above.
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-For $remote_addr;
        proxy_set_header X-Real-IP $remote_addr;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection $connection_upgrade;

        proxy_request_buffering off;
        proxy_buffering off;
        proxy_read_timeout 3600s;
        proxy_send_timeout 3600s;
    }
}
```

Use `$remote_addr` rather than `$proxy_add_x_forwarded_for`: the latter appends to a client-supplied `X-Forwarded-For` header, so the first address can still be forged. If the proxy runs in Docker, use the container address or bridge network in `proxy_pass` and add the matching source address to `security.trusted_proxies`.

## API

All application endpoints live under `/api/v1`; `/healthz` is unauthenticated. The OpenAPI contract is in [`docs/openapi.yaml`](docs/openapi.yaml).

## Security notes

- Passwords are hashed with Argon2.
- Session, API, and share tokens are stored as SHA-256 hashes; plaintext tokens are shown only at creation.
- All user filesystem access goes through the path resolver.
- Commands are typed filesystem operations, not shell execution.
- Share links are intentionally bearer secrets. Treat them as credentials.
- Never commit `config.json`, databases, logs, or generated credentials.

A dedicated non-root user is recommended. Grant only the data directories it needs through ownership, group membership, or ACLs. Running as root is common on personal NAS boxes, but then the OS provides no write protection:

- Add only the data directories you need; never expose `/`, `/etc`, or `/root` as writable roots.
- Mark system/config directories read-only inside FileHub.
- Do not expose FileHub directly to the public internet; use a VPN or reverse proxy with strong passwords.
- Keep snapshots/backups: a root-level delete has no permission safety net.
- Keep `ProtectSystem=strict` and a narrow `ReadWritePaths` list in the systemd unit.

As root, `access(W_OK)` reports `0555` directories as writable (root really can write), so filesystem read-only detection is largely ineffective; application-level read-only still works. Windows LocalSystem has the same caveat: mount only the data directories it needs.

## Development

```bash
make check   # Rust fmt/clippy, Svelte lint/typecheck, and tests
make test    # Rust and frontend tests
make build   # Linux musl release
make build-windows
```

Repository layout:

```text
src/          Rust backend and HTTP/WebSocket API
migrations/   Forward-only SQLite migrations
web/          Svelte 5 frontend
docs/         OpenAPI and operations notes
tests/        Rust integration tests
```

## Known limitations

- Non-UTF-8 filenames are not supported and are excluded from indexing/search.
- HEIC/HEIF/RAW thumbnails are not generated.
- Video thumbnails require an optional system `ffmpeg`.
- Windows USN incremental indexing is implemented but should be validated against the target NTFS volume before production rollout.
- FileHub is designed for a single application instance per database.

## License

MIT. See [`LICENSE`](LICENSE).
