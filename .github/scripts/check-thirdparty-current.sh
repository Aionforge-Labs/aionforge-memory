#!/usr/bin/env bash
# Verify THIRDPARTY.md is in sync with Cargo.lock by regenerating with
# cargo-about and diffing. Drift indicates a dependency was added/changed
# without updating the attribution file, or attribution was hand-edited.
# Third-party attribution drift blocks merge (AGENTS.md).

set -euo pipefail

if ! command -v cargo-about >/dev/null 2>&1; then
  echo "cargo-about not found. Install with: cargo install cargo-about --features cli" >&2
  exit 2
fi

GENERATED=$(mktemp)
trap 'rm -f "$GENERATED"' EXIT

cargo about generate about.hbs > "$GENERATED"

if ! diff -q THIRDPARTY.md "$GENERATED" >/dev/null 2>&1; then
  echo "FAIL: THIRDPARTY.md is out of sync with Cargo.lock."
  echo "Regenerate with: cargo about generate about.hbs > THIRDPARTY.md"
  echo
  echo "Diff:"
  diff THIRDPARTY.md "$GENERATED" || true
  exit 1
fi

echo "OK: THIRDPARTY.md is in sync with Cargo.lock."
