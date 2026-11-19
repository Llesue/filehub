#!/usr/bin/env bash
# Build the Linux musl release on a remote host and fetch the binary back.
#
# Requirements:
#   - `web/dist` exists locally (run `pnpm build` in web/ first)
#   - remote host has Rust + the x86_64-unknown-linux-musl target
#   - passwordless SSH and rsync
#
# Environment overrides:
#   FILEHUB_BUILD_HOST  SSH alias/host (default: parasite6)
#   FILEHUB_BUILD_DIR   remote workspace (default: /root/build/filehub)
#   FILEHUB_BUILD_JOBS  cargo jobs (default: 6)
#
# The remote target directory is preserved between builds for incremental
# compiles; source files are synced with --delete.
set -euo pipefail

HOST="${FILEHUB_BUILD_HOST:-parasite6}"
REMOTE_DIR="${FILEHUB_BUILD_DIR:-/root/build/filehub}"
JOBS="${FILEHUB_BUILD_JOBS:-6}"
TARGET="x86_64-unknown-linux-musl"
TARGET_ENV="${TARGET//-/_}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SSH_CMD="ssh -o BatchMode=yes"

if [[ ! -f "$ROOT/web/dist/index.html" ]]; then
  echo "web/dist is missing; run 'pnpm build' in web/ first" >&2
  exit 1
fi

echo "syncing sources to $HOST:$REMOTE_DIR"
ssh "$HOST" "mkdir -p '$REMOTE_DIR'"
rsync -a --delete -e "$SSH_CMD" \
  --exclude target \
  --exclude node_modules \
  --exclude .git \
  --exclude data \
  "$ROOT/" "$HOST:$REMOTE_DIR/"

echo "building on $HOST (jobs=$JOBS)"
ssh "$HOST" "cd '$REMOTE_DIR' && \
  export PATH=/root/.cargo/bin:\$PATH && \
  export CARGO_NET_OFFLINE=true && \
  export CC_${TARGET//-/_}=musl-gcc && \
  export CARGO_TARGET_${TARGET_ENV^^}_LINKER=musl-gcc && \
  CARGO_BUILD_JOBS=$JOBS cargo build --release --locked --target $TARGET"

mkdir -p "$ROOT/target/remote"
rsync -a -e "$SSH_CMD" \
  "$HOST:$REMOTE_DIR/target/$TARGET/release/filehub" \
  "$ROOT/target/remote/filehub-linux-$TARGET"

BIN="$ROOT/target/remote/filehub-linux-$TARGET"
chmod +x "$BIN"
echo "artifact: $BIN"
"$BIN" --version
sha256sum "$BIN"
