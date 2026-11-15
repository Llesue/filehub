# FileHub 运维手册

## 部署

```bash
make build
sudo install -m 0755 target/x86_64-unknown-linux-musl/release/filehub /usr/local/bin/filehub
sudo mkdir -p /etc/filehub /var/lib/filehub /var/cache/filehub /srv/filehub/users
sudo cp config.example.json /etc/filehub/config.json
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
sqlite3 /var/lib/filehub/filehub.db ".backup '/backup/filehub-$(date +%F).db'"
```

或：

```bash
sqlite3 /var/lib/filehub/filehub.db "VACUUM INTO '/backup/filehub-$(date +%F).db'"
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
