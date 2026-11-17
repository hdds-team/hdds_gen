#!/usr/bin/env bash
# SPDX-License-Identifier: MIT

set -euo pipefail

echo "[fmt]"
cargo fmt --all -- --check

echo "[clippy]"
cargo clippy --all-targets -- -D warnings

echo "[test]"
cargo test --all --locked

echo "[idl: examples]"
for f in examples/*.idl; do
  echo "Check $f"
  cargo run --quiet --bin hddsgen -- check "$f"
done

echo "[idl: canonical fmt strict]"
for f in examples/canonical/*.idl; do
  tmp=$(mktemp)
  cargo run --quiet --bin hddsgen -- fmt "$f" > "$tmp"
  sed -n '1p' "$f" | grep -qx '// SPDX-License-Identifier: Apache-2.0 OR MIT'
  sed -n '2p' "$f" | grep -qx '// Copyright (c) 2025-2026 naskel.com'
  sed -n '3p' "$f" | grep -qx ''
  diff -u <(tail -n +4 "$f") "$tmp"
  rm -f "$tmp"
done

echo "OK"
