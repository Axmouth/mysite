#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "${ROOT_DIR}"

cargo fmt --check
node --check static/theme.js
node --check static/editor.js
cargo check --locked
cargo test --locked
scripts/smoke-test.sh
