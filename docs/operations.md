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

使用专用非 root 用户运行，确保 `users_root`、`db_path` 父目录、`cache_dir` 可写。

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

[Install]
WantedBy=multi-user.target
```

日志输出到 stdout/stderr，由 journald 收集：

```bash
journalctl -u filehub -f
```

## 反向代理

FileHub 默认监听 HTTP，建议由 Caddy/Nginx 终止 TLS。反向代理需：

- 透传 `Host`、`X-Forwarded-For`、`X-Forwarded-Proto`
- 请求体上限至少覆盖最大分片（默认 8MiB，留余量）
- 下载/ZIP 关闭代理缓冲，避免大文件卡住
- HTTPS 下把 `security.cookie_secure` 设为 `true`

## 备份与恢复

SQLite 使用 WAL，不能只复制 `.db` 文件。推荐：

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
- 缩略图不显示：确认 `thumbnails.enabled`，HEIC/RAW/视频可能按设计降级为图标

## 后台任务与目录大小

目录大小、复制/移动、索引和缩略图都由进程内后台任务执行，HTTP 请求只负责入队和查询状态。

- `jobs.io_workers` 控制目录大小、索引和复制/移动共享的磁盘并发，默认 `1`
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
- `thumbnails.video_timeout_secs` 现在会真正限制 ffmpeg 抽帧时间，超时会 kill 进程
- 审计日志由后台批量写入（最多 200 条或 1 秒），服务优雅关闭时会 flush
- Windows 上优先使用 MFT + USN Journal 增量；USN 不可用/回退时自动走 MFT/目录扫描。USN 状态按 volume 存在 `usn_state` 表
