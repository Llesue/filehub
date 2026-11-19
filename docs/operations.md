# FileHub 运维手册

## 部署

```bash
# 推荐从 GitHub Releases 下载对应架构的静态二进制；也可在源码目录执行 make build。
chmod +x filehub-*-x86_64-unknown-linux-musl
sudo install -m 0755 filehub-*-x86_64-unknown-linux-musl /usr/local/bin/filehub
sudo useradd --system --create-home --home-dir /home/filehub --shell /usr/sbin/nologin filehub
sudo mkdir -p /etc/filehub /home/filehub/users /home/filehub/cache
sudo cp config.example.json /etc/filehub/config.json
sudo chown -R filehub:filehub /home/filehub
```

使用专用非 root 用户运行，确保 `users_root`、`db_path` 父目录、`cache_dir` 可写。这些目录缺失时会在启动或 `reload_settings` 时自动创建（Unix 下 `users_root`/`cache_dir` 会收紧到 `0700`），但属主仍需提前配置；外部存储根不会被自动创建。

### 外部目录权限

FileHub 的“可写”是应用层判断（root 配置、用户授权）和**服务进程的系统权限**取交集。非 root 运行时，外部目录必须对服务账号可写，否则列表会直接显示“只读（系统）”，写操作返回 `403 PERMISSION_DENIED`。

推荐用属组或 ACL 授权，而不是 `chmod 777`：

```bash
# 属组方式
sudo chgrp -R filehub /mnt/media/photos
sudo chmod -R g+rwX /mnt/media/photos
sudo find /mnt/media/photos -type d -exec chmod g+s {} +

# ACL 方式（不改变原属组）
sudo setfacl -R -m u:filehub:rwX /mnt/media/photos
sudo setfacl -R -d -m u:filehub:rwX /mnt/media/photos
```

目录需要同时具备 `w` 和 `x` 才能在内部创建、删除、重命名条目；只读目录用 `chmod 0555` 即可。FileHub 每次列目录都会探测当前目录，权限变更无需重启即可生效；父列表不会逐个探测子目录，进入子目录后才判定。

**root 运行**：`access(W_OK)` 对 `0555` 目录也会返回可写（root 确实能写），文件系统只读探测基本失效；只读挂载仍能通过 `statvfs` 识别。应用层只读仍然有效，但没有任何系统权限兜底，误删风险由 root 自己承担。保留 `ProtectSystem=strict` + `ReadWritePaths` 限制可写范围，并且不要把 `/`、`/etc`、`/root` 整体添加为读写目录。

**Windows**：无法可靠探测目录 ACL 写权限，列表只反映应用层只读；写失败时统一返回 `403 PERMISSION_DENIED`。以 LocalSystem 运行时同样只挂载必要的数据目录。

## systemd

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
RestartSec=2
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

加固项说明：

- `NoNewPrivileges=true`：禁止服务通过 setuid/setgid 程序或 capability 提权
- `PrivateTmp=true`：服务使用独立的 `/tmp`，看不到宿主机其他进程的临时文件
- `ProtectSystem=strict`：整个文件系统只读，仅 `/dev`、`/proc`、`/sys` 保持可写；配置目录 `/etc/filehub` 因此只读，符合运行需要
- `ReadWritePaths=/home/filehub`：只放开数据目录写权限，覆盖 `users_root`、`cache_dir` 和 `db_path` 的父目录；如果这些路径配置到别处，需要逐个加入
- `UMask=0077`：服务创建的新文件默认仅属主可读写，避免上传文件和临时文件被其他用户读取
- `RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX`：只允许 IPv4、IPv6 和 Unix socket，禁止其他地址族
- `CapabilityBoundingSet=`：清空 capability 集合，服务不保留任何 Linux capability

不要设置 `ProtectHome=yes`：systemd 会隐藏整个 `/home`，`/home/filehub` 下的数据对服务不可见，启动、索引、上传都会失败。也不能用 `ProtectHome=read-only`，因为 FileHub 必须写数据目录；保持 `ProtectHome` 未设置即可。

日志输出到 stdout/stderr，由 journald 收集：

```bash
journalctl -u filehub -f
```

## Windows 服务日志

Windows 服务日志按天写入 `%ProgramData%\FileHub\filehub.log.<date>`（例如 `filehub.log.2026-11-16`）。`log.retention_days` 控制保留天数：默认 `14`，`0` 表示不清理，最大 `3650`。服务启动时和每小时维护任务会删除超过保留期的 `filehub.log.*` 文件。Linux 前台或 systemd 运行时不走这条路径，日志仍由 journald 管理。

## 反向代理

不要把 FileHub 直接暴露到公网。由 Caddy 或 Nginx 终止 TLS，再通过回环地址或私有网络把流量转发给 FileHub。反向代理和 FileHub 在同一台机器时，建议把 `listen` 设为 `127.0.0.1:8080`，避免绕过代理直接访问。

代理必须保留原始 `Host`，并转发客户端地址 `X-Forwarded-For`、`X-Real-IP` 和原始协议 `X-Forwarded-Proto`（`http`/`https`）。这些头必须由代理覆盖，不能追加客户端预置的值，否则客户端可以伪造审计 IP。

FileHub 只在对端地址命中 `security.trusted_proxies` 时才信任转发头。把代理连接 FileHub 时使用的源地址填进去（`127.0.0.1`、`::1` 或 Docker 网桥地址/网段）。不匹配时，审计日志记录的是代理地址。公网使用 HTTPS 时把 `security.cookie_secure` 设为 `true`；如果仍在用 HTTP 测试却开启它，浏览器不会保存会话 Cookie。

```json
{
  "security": {
    "cookie_secure": true,
    "trusted_proxies": ["127.0.0.1", "::1"]
  }
}
```

反向代理配置要点：

- WebSocket：`/api/v1/terminal/ws` 需要 HTTP/1.1 以及 `Upgrade`/`Connection` 头。Caddy 自动处理；Nginx 需要下面的 `map` 块。
- 上传请求体上限：要大于 `uploads.chunk_size`（默认 8 MiB），示例给 32 MiB；调大分片后同步调大。
- 下载/ZIP 流式响应：关闭代理缓冲（Nginx `proxy_buffering off` 与 `proxy_request_buffering off`，Caddy `flush_interval -1`）。下载不发送 `X-Accel-Buffering`；实时事件流（`/api/v1/events`、公开分享事件流）会发送 `X-Accel-Buffering: no`，但仍建议在代理侧关闭缓冲。
- 长下载/大 ZIP：示例给 1 小时读写超时；如果归档可能更久，继续调大，或降低 `downloads.zip_max_bytes`。

### Caddy

```caddyfile
files.example.com {
    request_body {
        max_size 32MB
    }

    # WebSocket（包括 /api/v1/terminal/ws）由 reverse_proxy 自动转发。
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

Caddy 会自动申请 TLS 证书，`reverse_proxy` 默认支持 WebSocket 升级。`header_up` 会覆盖客户端预置的转发头。

### Nginx

把升级映射放在 `http` 上下文（例如 `/etc/nginx/conf.d/filehub-upgrade.conf`），然后使用下面的 `server` 块：

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

        # WebSocket：/api/v1/terminal/ws 使用上面的 upgrade map。
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

`$proxy_add_x_forwarded_for` 会追加客户端预置的 XFF，第一个地址仍可伪造，所以这里必须用 `$remote_addr` 覆盖。代理运行在 Docker 中时，`proxy_pass` 使用容器地址或网桥地址，并把对应的源地址加入 `security.trusted_proxies`。

## 备份与恢复

SQLite 使用 WAL：正常运行时最近的写入可能还在 `filehub.db-wal` 中（自动 checkpoint 阈值 1000 页，WAL 上限约 64MB）。优雅关闭会执行 `wal_checkpoint(TRUNCATE)` 并清空 WAL；硬崩溃后残留的 WAL 会在下次启动时自动恢复。不能只复制 `.db` 文件。推荐：

```bash
sqlite3 /home/filehub/filehub.db ".backup '/backup/filehub-$(date +%F).db'"
```

或：

```bash
sqlite3 /home/filehub/filehub.db "VACUUM INTO '/backup/filehub-$(date +%F).db'"
```

恢复：停止服务 → 用备份替换 `db_path` → 删除旧 `-wal`/`-shm` → 启动。用户文件不在 DB 中，需单独用文件系统快照/rsync 备份 `users_root`。

## 升级

1. 备份 DB 与 `users_root`
2. 替换二进制
3. 启动时自动执行 forward-only 迁移
4. 检查 `journalctl` 与 `/healthz`

## 故障排查

- `503 frontend not built`：发布时未先执行前端构建，重新 `make build`
- 启动失败并提示 users_root：显式配置的目录必须存在且为目录
- 登录 429：同一用户名 5 次失败后锁定 5 分钟
- 上传 507：用户配额不足
- 上传 429：同一用户未完成的会话超过 `uploads.concurrency_per_user`（默认 `32`，暂停的上传也会占用会话），等待过期或取消旧上传
- 上传支持分片续传：刷新页面后重新选择相同文件会复用服务端会话并跳过已收分片；主动取消会删除临时文件
- 0 字节文件可以上传；Windows 文件名按 UTF-16 长度校验
- 缩略图不显示：确认 `thumbnails.enabled`，HEIC/RAW/视频可能按设计降级为图标

## 后台任务与目录大小

目录大小、复制/移动、索引和缩略图都由进程内后台任务执行，HTTP 请求只负责入队和查询状态。

- `jobs.io_workers` 控制目录大小和索引等后台维护任务的磁盘并发，默认 `1`
- `jobs.interactive_workers` 控制复制/移动等交互任务的并发，默认 `2`；与后台维护任务分开，避免移动任务长时间停在“排队中”
- `jobs.dir_size_workers` 控制目录大小扫描的并发，默认 `1`；与索引任务分开，索引大目录时目录大小仍能刷新
- `jobs.queue_max` 是全局等待队列上限，默认 `256`
- `jobs.dir_size_ttl_seconds` 控制目录大小缓存有效期，默认 `300` 秒
- `jobs.dir_size_use_logical` 默认 `true`（逻辑大小）；设为 `false` 使用磁盘块大小并在 Unix 上按 inode 去重硬链接
- `index.scheduled_scans` 启用后台索引维护，默认 `true`
- `index.quick_scan_minutes` 只 stat 已知目录 mtime，变化才重读，默认 `10` 分钟
- `index.full_scan_hours` 周期性完整重建索引，默认 `24` 小时
- `index.auto_index_on_browse` 在浏览到未索引目录时后台补一次非递归索引，默认 `true`
- 扫描父目录时会同时写入其所有子目录的大小，进入子目录不再重复扫描
- 服务重启后已完成的目录大小仍可用；未完成的后台任务会标记为失败，需要时自动重新入队
- 复制/移动任务可通过 `DELETE /api/v1/jobs/{id}` 取消，已写入的部分目标会被清理

管理页的“系统状态”会显示后台任务队列、目录大小缓存状态、数据库/WAL 大小和最近慢请求。

## 审计与数据保留

- `jobs.audit_retention_days`：审计日志保留天数，默认 `90`
- `jobs.audit_max_rows`：审计日志最大条数，默认 `200000`
- `jobs.job_retention_days`：已完成任务保留天数，默认 `30`
- 目录大小历史行保留 30 天，由每小时维护任务清理
- 每小时执行一次 WAL checkpoint；备份时仍应使用 `.backup` 或 `VACUUM INTO`

## 索引

- 索引任务在后台队列执行，可按 `GET /api/v1/jobs/{id}` 查看进度并通过 `DELETE` 取消
- 父根覆盖子根时只索引父根，管理页会在子根旁显示“已被父根覆盖”
- `indexed_dirs.total_size_bytes` 保存完整索引的子树大小；部分索引不会当作最终大小使用
- 索引条目保存 `mime_type`、`created_at`、`has_preview`、`is_hidden`，列表和缩略图不再只靠扩展名猜测
- 搜索结果支持 `min_size`、`max_size`、`modified_after`、`modified_before`、`is_dir`、`kind`
- 缩略图失败会写 `.fail` 负缓存标记，同一文件/尺寸不会重复解码；成功结果支持 `ETag`/`If-None-Match` 返回 304
- `thumbnails.preview_workers`（默认 `1`）为预览大图保留独立并发；列表小图使用 `workers - preview_workers` 的通道，滚动目录不会阻塞预览。预览任务还有少量保留队列名额，避免目录缩略图占满队列后返回 503
- `thumbnails.video_timeout_secs` 现在会真正限制 ffmpeg 抽帧时间，超时会 kill 进程
- 索引条目按 500 条一批提交，避免单个大目录长时间持有 SQLite 写锁；数据库连接池优先使用空闲连接
- 路径缓存失效会合并为一个事务；删除超过 500ms 会记录 `slow delete request`，包含 resolve/remove/cache/audit 各阶段耗时
- 审计日志由后台批量写入（最多 200 条或 1 秒），服务优雅关闭时会 flush
- Windows 上优先使用 MFT + USN Journal 增量；USN 不可用/回退时自动走 MFT/目录扫描。USN 状态按 volume 存在 `usn_state` 表
