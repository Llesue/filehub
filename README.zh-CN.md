# FileHub

[English](README.md) | 简体中文

FileHub 是一个单可执行文件的自托管文件管理器。它在浏览器里提供文件浏览、断点续传、分享、搜索、缩略图和管理后台，运行时不需要 Docker、Node 或独立数据库服务。

前端是嵌入 Rust 二进制的 Svelte 5 SPA，元数据存放在 SQLite，文件仍然在文件系统里。Linux 发布物是单个静态 musl 二进制；Windows 使用同一套代码，单独产出 `filehub.exe`。

![FileHub 主界面](docs/assets/filehub-main-v2.png)

## 为什么做 FileHub

很多年前，我在 WD My Cloud 上第一次使用 FileBrowser，被它简洁优雅的界面和在低性能机器上的快速响应所打动。后来，我有了多用户鉴权、以及管理 NAS 上任意目录的需求，FileBrowser 渐渐无法满足我；又惊闻 FileBrowser 已宣布停止更新，于是萌生了一个想法：借助 OpenCode 和 DeepSeek，开发一个属于自己的 FileBrowser。

## 在线 Demo

https://filehub.ihtom.com

- 账号：`demo` / `filehub2026`
- 公开演示环境，每 24 小时重置一次
- 不要上传隐私或敏感数据，里面的文件一律视为不可信

## 使用场景

- 家庭 NAS 或外接硬盘的浏览器入口
- 手机 ↔ 电脑文件中转，支持断点续传
- 家庭相册/影视库，配合密码和有效期分享
- 旧硬盘只读档案库，避免误改原盘
- 把 Windows 机器临时当文件服务器
- 小团队/工作室文件柜：多用户、配额、外部路径
- 在浏览器里编辑服务器配置、日志和文档
- 用可写公开分享收集活动照片
- 低功耗 ARM 设备或单个静态 Linux 二进制部署
- 对接 HTTP API 的客户端与自动化

## 功能

- 浏览、上传、下载、重命名、复制、移动和删除文件/目录
- 分片续传上传、多文件流式 ZIP 下载、目录内 ZIP 打包/解压任务
- 目录索引和文件名全文搜索，搜索结果支持排序和多选
- 常见图片缩略图；安装 `ffmpeg` 后可生成视频缩略图
- 文件/目录公开分享：密码、有效期、只读、访问次数；可写目录分享支持上传
- 在线文本编辑器、ETag 乐观锁、Markdown 预览
- 每用户偏好、头像、配额、只读/读写存储根
- 管理后台：用户、外部路径、配额、按用户功能开关、系统设置和审计日志
- WebSocket 受限命令面板：`help`、`ls`、`tree`、`cp`、`mv`、`rm`、`zip`、`unzip`、`job`、`roots`，高危删除带验证码
- 联邦远程目录：把另一个 FileHub 实例授权的目录挂载进来，支持只读/读写、范围令牌、邀请信息、按用户开关，以及代理搜索、缩略图、音频和 ZIP
- Linux 单文件部署；Windows 支持 LocalSystem 服务

## 快速开始

### Linux

推荐直接从 [GitHub Releases](https://github.com/Llesue/filehub/releases) 下载静态发布版。按架构选择 `x86_64` 或 `aarch64` 的 musl 文件，先用对应的 `.sha256` 校验，再安装：

```bash
chmod +x filehub-*-x86_64-unknown-linux-musl
sudo install -m 0755 filehub-*-x86_64-unknown-linux-musl /usr/local/bin/filehub
```

如果要从源码构建：

```bash
make setup
make build
sudo install -m 0755 target/x86_64-unknown-linux-musl/release/filehub /usr/local/bin/filehub
```

创建服务用户、配置和数据目录。配置文件固定在 `/etc/filehub`，FileHub 的运行数据放在 `/home/filehub`：

```bash
sudo useradd --system --create-home --home-dir /home/filehub --shell /usr/sbin/nologin filehub
sudo mkdir -p /etc/filehub /home/filehub/users /home/filehub/cache
sudo cp config.example.json /etc/filehub/config.json
sudo chown -R filehub:filehub /home/filehub
```

数据目录（`/home/filehub`、`users`、`cache`）缺失时会在启动时自动创建；仍需提前把属主和权限配置好。

前台运行：

```bash
filehub --config /etc/filehub/config.json
```

systemd 示例：

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

`NoNewPrivileges` 和空的 `CapabilityBoundingSet` 阻止提权和 capability 提升，`PrivateTmp` 隔离 `/tmp`，`ProtectSystem=strict` 让整个文件系统只读，`ReadWritePaths` 只放开 `/home/filehub` 的写权限，`UMask=0077` 保证新文件仅属主可读写，`RestrictAddressFamilies` 只允许 IP 和 Unix socket。如果 `users_root`、`cache_dir` 或 `db_path` 不在 `/home/filehub` 下，需要把对应路径加入 `ReadWritePaths`（`db_path` 用其父目录）。不要设置 `ProtectHome=yes`：它会隐藏 `/home`，导致服务无法访问 `/home/filehub`。

默认监听 `http://<host>:8080/`。首次启动会在日志里打印 setup token（也可通过 `FILEHUB_SETUP_TOKEN` 指定），打开页面后用它创建第一个管理员。

### Windows

推荐从 [GitHub Releases](https://github.com/Llesue/filehub/releases) 下载 Windows 可执行文件（`filehub-<version>-x86_64-pc-windows-msvc.exe`），校验对应的 `.sha256` 后复制到 `C:\Program Files\FileHub\filehub.exe`。

如需在 Linux 上交叉编译：

```bash
cargo install cargo-xwin --locked
make build-windows
```

产物是 `target/x86_64-pc-windows-msvc/release/filehub.exe`。复制到 Windows 后，用管理员 PowerShell 安装服务：

```powershell
& "C:\Program Files\FileHub\filehub.exe" service install
sc.exe start FileHub
```

服务以 `LocalSystem` 运行，开机自启；数据库、用户目录、缓存和日志默认在 `C:\ProgramData\FileHub`。服务日志按天写入 `%ProgramData%\FileHub\filehub.log.<date>`（例如 `filehub.log.2026-11-16`）。`log.retention_days` 控制保留天数：默认 `14`，`0` 表示不清理，最大 `3650`；过期日志会在启动时和每小时维护任务中删除。

读取服务日志和首次初始化 token：

```powershell
Get-ChildItem C:\ProgramData\FileHub\filehub.log.* |
  Sort-Object LastWriteTime |
  Select-Object -Last 1 |
  Get-Content -Tail 50
```

停止和卸载：

```powershell
sc.exe stop FileHub
& "C:\Program Files\FileHub\filehub.exe" service uninstall
```

Windows 索引优先使用 NTFS MFT，之后通过 USN Journal 做增量更新；USN 不可用时回退普通目录扫描。

## 配置

`config.json` 提供部署级默认值；后台修改的设置写入 SQLite，并优先于配置文件。

最重要的字段：

```json
{
  "listen": "0.0.0.0:8080",
  "db_path": "/home/filehub/filehub.db",
  "users_root": "/home/filehub/users",
  "cache_dir": "/home/filehub/cache"
}
```

完整配置见 [`config.example.json`](config.example.json)。Windows 默认使用 `C:\ProgramData\FileHub`；安装服务时可以显式指定配置：

```powershell
& "C:\Program Files\FileHub\filehub.exe" --config "C:\ProgramData\FileHub\config.json" service install
```

不要把 FileHub 直接暴露到公网。建议放在 TLS 反向代理后面，开启 `security.cookie_secure`，并限制数据目录权限。

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

## API

所有业务接口前缀为 `/api/v1`，`/healthz` 不需要认证。接口契约见 [`docs/openapi.yaml`](docs/openapi.yaml)。

## 安全说明

- 密码使用 Argon2 哈希
- session、API Token、分享 Token 都只存 SHA-256 哈希；明文只在创建时返回
- 所有用户文件访问都经过路径解析器
- 命令面板是类型化文件操作，不执行 shell
- 分享链接本身就是凭证，请按密码级别保护
- 不要提交 `config.json`、数据库、日志或生成的凭证

推荐使用专用非 root 用户运行，只通过属主、属组或 ACL 授予必要的数据目录。个人 NAS 上以 root 运行很常见，但系统不会提供写入保护：

- 只添加需要的数据目录，不要把 `/`、`/etc`、`/root` 整体加成读写
- 系统目录（配置、程序）在 FileHub 里设为只读
- 不要直接暴露公网；至少走 VPN 或反代 + 强密码
- 保留快照/备份，root 下的误删没有权限兜底
- systemd 继续使用 `ProtectSystem=strict` 和尽量窄的 `ReadWritePaths`

root 下 `access(W_OK)` 对 `0555` 目录也会返回可写（root 确实能写），文件系统只读探测基本失效；应用层只读仍然有效。Windows LocalSystem 同理：只挂必要的数据目录。

## 开发

```bash
make check   # Rust fmt/clippy、Svelte lint/typecheck 和测试
make test    # Rust 与前端测试
make build   # Linux musl release
make build-windows
```

目录：

```text
src/          Rust 后端和 HTTP/WebSocket API
migrations/   只向前迁移的 SQLite schema
web/          Svelte 5 前端
docs/         OpenAPI 与运维文档
tests/        Rust 集成测试
```

## 已知边界

- 不支持非 UTF-8 文件名，也不进入索引/搜索
- 不生成 HEIC/HEIF/RAW 缩略图
- 视频缩略图依赖可选的系统 `ffmpeg`
- Windows USN 增量索引已实现，但上线前应先在目标 NTFS 卷上验证 rename/delete/USN 失效回退场景。
- 每个数据库只支持一个 FileHub 实例

## 许可证

MIT，详见 [`LICENSE`](LICENSE)。
