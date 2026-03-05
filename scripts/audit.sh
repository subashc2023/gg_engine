#!/usr/bin/env bash
# Run dependency security audit.
# Install: cargo install cargo-audit
# Usage:   ./scripts/audit.sh

set -euo pipefail

if ! command -v cargo-audit &> /dev/null; then
    echo "cargo-audit not found. Install with: cargo install cargo-audit"
    exit 1
fi

echo "=== Running cargo audit ==="
cargo audit

echo ""
echo "=== Checking for yanked crates ==="
cargo audit --deny yanked 2>/dev/null || true
