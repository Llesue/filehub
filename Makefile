SHELL := /bin/sh
CARGO ?= cargo
PNPM ?= pnpm
WEB := web
TARGET := x86_64-unknown-linux-musl
WINDOWS_TARGET := x86_64-pc-windows-msvc
RELEASE_BIN := target/$(TARGET)/release/filehub
WINDOWS_BIN := target/$(WINDOWS_TARGET)/release/filehub.exe

.PHONY: help setup web-install web-build fmt check test build build-windows release bump-version dev audit clean

help:
	@printf '%s\n' \
		'make setup      安装工具链与依赖' \
		'make dev        同时启动后端与 Vite' \
		'make fmt        格式化 Rust 与前端' \
		'make check      fmt/clippy/前端 lint/typecheck/test' \
		'make test       Rust 测试 + 前端测试' \
		'make build      构建 musl 单文件' \
		'make build-windows  交叉构建 Windows exe' \
		'make release    check + test + build + SHA-256' \
		'make bump-version  修改 Cargo/package 版本号（minor）' \
		'make audit      依赖与许可证审计' \
		'make clean      清理构建产物'

setup: web-install
	rustup component add clippy rustfmt
	rustup target add $(TARGET)
	$(CARGO) fetch

web-install:
	cd $(WEB) && $(PNPM) install

web-build: web-install
	cd $(WEB) && $(PNPM) build

fmt: web-install
	$(CARGO) fmt --all
	cd $(WEB) && $(PNPM) format

check: web-install
	$(CARGO) fmt --all -- --check
	$(CARGO) clippy --all-targets --all-features -- -D warnings
	cd $(WEB) && $(PNPM) lint
	cd $(WEB) && $(PNPM) check
	cd $(WEB) && $(PNPM) test

test: web-install
	$(CARGO) test --all-targets
	cd $(WEB) && $(PNPM) test

build: web-build
	$(CARGO) build --release --locked --target $(TARGET)
	@ls -lh $(RELEASE_BIN)

build-windows: web-build
	cargo xwin build --release --locked --target $(WINDOWS_TARGET)
	@ls -lh $(WINDOWS_BIN)

bump-version:
	node scripts/bump-version.mjs

release: check test build
	@cd target/$(TARGET)/release && sha256sum filehub > filehub.sha256
	@cat target/$(TARGET)/release/filehub.sha256

dev: web-install
	@trap 'kill 0' INT TERM EXIT; \
	(cd $(WEB) && $(PNPM) dev) & \
	$(CARGO) run & \
	wait

audit:
	@command -v cargo-deny >/dev/null 2>&1 || { echo "cargo-deny is not installed"; exit 1; }
	$(CARGO) deny check
	$(CARGO) audit
	cd $(WEB) && $(PNPM) audit --prod

clean:
	$(CARGO) clean
	rm -rf $(WEB)/dist
