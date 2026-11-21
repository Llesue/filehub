#!/usr/bin/env bash
# Build a release on a remote host and fetch the artifact back.
#
# Requirements:
#   - `web/dist` exists locally (run `pnpm build` in web/ first)
#   - remote host has the Rust toolchain and the selected target
#   - Linux: x86_64-unknown-linux-musl + musl-gcc
#   - Windows: x86_64-pc-windows-msvc + cargo-xwin + clang/lld + SDK cache
#   - passwordless SSH and rsync
#
# Usage:
#   ./scripts/build-remote.sh           # Linux musl
#   ./scripts/build-remote.sh windows   # Windows MSVC
#
# Environment overrides:
#   FILEHUB_BUILD_HOST  SSH alias/host (default: parasite6)
#   FILEHUB_BUILD_DIR   remote workspace (default: /root/build/filehub)
#   FILEHUB_BUILD_JOBS  cargo jobs (default: 6)
#
# The remote target directory is preserved between builds for incremental
# compiles; source files are synced with --delete.
set -euo pipefail

MODE="${1:-linux}"
HOST="${FILEHUB_BUILD_HOST:-parasite6}"
REMOTE_DIR="${FILEHUB_BUILD_DIR:-/root/build/filehub}"
DEFAULT_JOBS="${FILEHUB_BUILD_JOBS:-}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SSH_CMD="ssh -o BatchMode=yes"

case "$MODE" in
  linux)
    TARGET="x86_64-unknown-linux-musl"
    TARGET_ENV="${TARGET//-/_}"
    ARTIFACT="filehub"
    LOCAL_ARTIFACT="filehub-linux-$TARGET"
    BUILD_CMD="cargo build --release --locked --target $TARGET"
    BUILD_ENV="export CC_${TARGET_ENV}=musl-gcc && export CARGO_TARGET_${TARGET_ENV^^}_LINKER=musl-gcc &&"
    ;;
  windows)
    TARGET="x86_64-pc-windows-msvc"
    ARTIFACT="filehub.exe"
    LOCAL_ARTIFACT="filehub-windows-x86_64.exe"
    BUILD_CMD="cargo xwin build --release --locked --target $TARGET"
    BUILD_ENV=""
    ;;
  *)
    echo "usage: $0 [linux|windows]" >&2
    exit 2
    ;;
esac

if [[ ! -f "$ROOT/web/dist/index.html" ]]; then
  echo "web/dist is missing; run 'pnpm build' in web/ first" >&2
  exit 1
fi

if [[ -z "$DEFAULT_JOBS" ]]; then
  read -r MEM_KB MAX_KB NPROC <<<"$(ssh "$HOST" "awk '/MemTotal/{m=\$2} END{print m}' /proc/meminfo; \
    dmesg 2>/dev/null | sed -n 's/.*Max. dynamic memory size: \([0-9]*\) MB.*/\1/p' | tail -1; \
    nproc" | tr '\n' ' ')"
  MAX_KB=$(( ${MAX_KB:-0} * 1024 ))
  NPROC="${NPROC:-1}"
  # Hyper-V dynamic memory may report only the current balloon size; use the
  # configured maximum when it is available.
  [[ "$MAX_KB" -gt "$MEM_KB" ]] && MEM_KB="$MAX_KB"
  # Keep roughly 1.25 GiB per rustc; the linker needs headroom too.
  JOBS=$(( MEM_KB / 1250000 ))
  [[ "$JOBS" -gt "$NPROC" ]] && JOBS="$NPROC"
  [[ "$JOBS" -lt 1 ]] && JOBS=1
  [[ "$JOBS" -gt 8 ]] && JOBS=8
else
  JOBS="$DEFAULT_JOBS"
fi

echo "syncing sources to $HOST:$REMOTE_DIR"
ssh "$HOST" "mkdir -p '$REMOTE_DIR'"
rsync -a --delete -e "$SSH_CMD" \
  --exclude target \
  --exclude node_modules \
  --exclude .git \
  --exclude data \
  "$ROOT/" "$HOST:$REMOTE_DIR/"

echo "building $MODE on $HOST (jobs=$JOBS)"
ssh "$HOST" "cd '$REMOTE_DIR' && \
  export PATH=/root/.cargo/bin:\$PATH && \
  export CARGO_NET_OFFLINE=true && \
  $BUILD_ENV \
  CARGO_BUILD_JOBS=$JOBS $BUILD_CMD"

mkdir -p "$ROOT/target/remote"
rsync -a -e "$SSH_CMD" \
  "$HOST:$REMOTE_DIR/target/$TARGET/release/$ARTIFACT" \
  "$ROOT/target/remote/$LOCAL_ARTIFACT"

BIN="$ROOT/target/remote/$LOCAL_ARTIFACT"
echo "artifact: $BIN"
if [[ "$MODE" == linux ]]; then
  chmod +x "$BIN"
  "$BIN" --version
fi
file "$BIN"
sha256sum "$BIN"
