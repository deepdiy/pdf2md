#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST_DIR="$ROOT_DIR/dist"
BIN_NAME="pdf2md"
CARGO_ZIGBUILD="${CARGO_ZIGBUILD:-$HOME/.cargo/bin/cargo-zigbuild}"
MINGW_PREFIX="${MINGW_PREFIX:-/opt/homebrew/Cellar/mingw-w64/14.0.0}"
MINGW_INCLUDE="$MINGW_PREFIX/toolchain-x86_64/x86_64-w64-mingw32/include"

cd "$ROOT_DIR"
mkdir -p "$DIST_DIR"

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "missing command: $1" >&2
    exit 1
  fi
}

require_file() {
  if [[ ! -e "$1" ]]; then
    echo "missing file: $1" >&2
    exit 1
  fi
}

copy_binary() {
  local src="$1"
  local dst="$2"

  require_file "$src"
  cp "$src" "$dst"
  chmod +x "$dst"
  ls -lh "$dst"
}

build_macos_arm64() {
  echo "==> build macOS Apple Silicon"
  cargo build --release --bin "$BIN_NAME"
  copy_binary \
    "$ROOT_DIR/target/release/$BIN_NAME" \
    "$DIST_DIR/pdf2md-macos-arm64"
}

build_linux_x64() {
  local target="x86_64-unknown-linux-gnu"

  echo "==> build Linux x86_64"
  "$CARGO_ZIGBUILD" build --release --bin "$BIN_NAME" --target "$target"
  copy_binary \
    "$ROOT_DIR/target/$target/release/$BIN_NAME" \
    "$DIST_DIR/pdf2md-$target"
}

build_linux_arm64() {
  local target="aarch64-unknown-linux-gnu"

  echo "==> build Linux ARM64"
  "$CARGO_ZIGBUILD" build --release --bin "$BIN_NAME" --target "$target"
  copy_binary \
    "$ROOT_DIR/target/$target/release/$BIN_NAME" \
    "$DIST_DIR/pdf2md-$target"
}

build_windows_x64() {
  local target="x86_64-pc-windows-gnu"

  echo "==> build Windows 10 x64"
  require_file "$MINGW_INCLUDE"
  env \
    "BINDGEN_EXTRA_CLANG_ARGS_${target}=--target=x86_64-w64-windows-gnu -isystem $MINGW_INCLUDE" \
    CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc \
    CXX_x86_64_pc_windows_gnu=x86_64-w64-mingw32-g++ \
    AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar \
    RANLIB_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ranlib \
    cargo build --release --bin "$BIN_NAME" --target "$target"
  copy_binary \
    "$ROOT_DIR/target/$target/release/$BIN_NAME.exe" \
    "$DIST_DIR/pdf2md-win10-x64.exe"
}

main() {
  require_cmd cargo
  require_cmd cp
  require_cmd chmod
  require_file "$CARGO_ZIGBUILD"

  build_macos_arm64
  build_linux_x64
  build_linux_arm64
  build_windows_x64

  echo "==> dist outputs"
  ls -lh "$DIST_DIR"
}

main "$@"
