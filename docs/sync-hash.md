# 同步内容哈希契约（SHA-256）

状态：0.2.6 / Gate 1–2
适用客户端：Android 0.1.5+

## 1. 能力发现

登录后 `GET /api/v1/me` 的 `capabilities`：

```json
{
  "hash": true,
  "hash_batch": true,
  "hash_upload_verify": true,
  "hash_verify_max_bytes": 67108864,
  "hash_algo": "sha256"
}
```

旧服务端没有该字段。客户端遇到 `/hash` 404、`HASH_UNSUPPORTED` 或缺少能力字段时，退回 `size + mtime` 判断，不阻塞同步。

## 2. 单文件哈希

```http
GET /api/v1/hash?root=2&path=Camera/IMG_001.jpg&wait=true&timeout_ms=5000
```

已就绪：

```json
{
  "path": "Camera/IMG_001.jpg",
  "algo": "sha256",
  "status": "ready",
  "sha256": "e3b0...",
  "size": 123456,
  "mtime": 1760000000,
  "cached": true,
  "computed_at": 1760000000
}
```

计算中：HTTP 202：

```json
{
  "path": "Camera/VID_001.mp4",
  "algo": "sha256",
  "status": "pending",
  "size": 123456789,
  "mtime": 1760000000,
  "retry_after_ms": 1000
}
```

- `wait=true` 最多等 5 秒；超时返回 202，不报错。
- 缓存命中要求 `size + mtime_ns + algo` 一致。
- `force=true` 忽略缓存。
- 文件在计算期间变化会重试，最多 2 次，之后返回 pending。

## 3. 批量哈希

```http
POST /api/v1/hashes
Content-Type: application/json

{ "root": 2, "paths": ["Camera/a.jpg", "Camera/b.mp4"], "force": false }
```

- 1–100 个路径，重复路径去重。
- 单个路径失败不影响其他路径。
- 响应顺序与请求一致。
- 小文件（≤8 MiB）通常同步返回 `ready`；其余返回 `pending`，客户端用同一路径轮询。
- API token 只读 scope 可用；session 请求需要 `X-CSRF-Token`。

响应：

```json
{
  "hashes": [
    { "path": "Camera/a.jpg", "status": "ready", "algo": "sha256", "sha256": "...", "size": 123, "mtime": 1760000000, "cached": true },
    { "path": "Camera/b.mp4", "status": "pending", "algo": "sha256", "size": 123456789, "mtime": 1760000000, "retry_after_ms": 1000 },
    { "path": "missing.jpg", "status": "error", "error": { "code": "NOT_FOUND", "message": "not found" } }
  ]
}
```

## 3.1 目录列表附带缓存哈希

`GET /api/v1/list?...&hashes=true` 只给已缓存且 `size/mtime` 匹配的文件附带 `sha256`，不触发计算。目录同步仍以 `POST /hashes` 为准，不要把列表哈希当成“所有文件都有哈希”。

## 4. 上传携带 SHA-256

`POST /api/v1/uploads` 增加可选字段：

```json
{
  "root": 2,
  "path": "Camera",
  "filename": "IMG_001.jpg",
  "total_size": 123456,
  "sha256": "64位小写hex",
  "verify": "auto"
}
```

- `verify`: `auto`（默认）/ `required` / `none`。
- `auto`：`total_size <= hash_verify_max_bytes` 时同步校验；超过阈值正常落盘，返回 `verified:false`、`hash_status:"too_large"`。
- `required`：超过阈值返回 413 `HASH_VERIFY_TOO_LARGE`，客户端应改用 `auto` 或先本地确认大小。
- `none`：不校验，不写哈希缓存。

`complete` 流程：

1. 服务端先校验临时文件 SHA-256，再处理冲突/finalize。
2. 不匹配：删除临时文件和上传会话，返回 422 `HASH_MISMATCH`，不碰目标文件。
3. 匹配：落盘，按最终路径的最终 `size + mtime_ns` 写入缓存。
4. 大文件 `verified:false` 时，客户端可再 `GET /hash` 比对。

`HASH_MISMATCH` 后必须重新创建 upload session，不能复用旧 session。

## 5. 同步流程

相册同步：

1. 本地计算 SHA-256（缓存 `contentHash/hashSize/hashMtime`）。
2. 目标路径已有文件 → `POST /hashes` 取远端哈希。
3. 相同 → `skipped_same_content`；不同或不存在 → 上传携带 `sha256`。
4. 小文件等待服务端校验结果；大文件接受 `verified:false`，可选 `GET /hash` 复核。
5. 全盘跨设备去重不在 0.1.5 承诺范围。

目录同步：

1. 远端快照仍用 `/list` 的 `size + mtime`。
2. 只对冲突候选/同 size 不同 mtime 的文件调用 `POST /hashes`。
3. 本地按需计算哈希；相同 → synced，不同 → 冲突或上传。
4. 下载后用 `GET /hash` 校验（可选）。

## 6. 降级矩阵

| 服务端返回 | 客户端行为 |
|---|---|
| 404 | 不支持，回退 size+mtime |
| 501 `HASH_UNSUPPORTED` | 远程根或不支持，回退 |
| 503 `HASH_QUEUE_FULL` | 暂时忙，退避重试，不切换算法 |
| 422 `HASH_MISMATCH` | 内容不一致，删除旧 upload session，重新上传 |
| 413 `HASH_*_TOO_LARGE` | 超过上限，改为不校验或分段策略 |

ETag 始终是 `size-mtime_ns`，不是内容哈希；下载缓存/断点续传不受影响。
