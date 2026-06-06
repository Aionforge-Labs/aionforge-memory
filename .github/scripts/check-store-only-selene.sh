#!/usr/bin/env bash
# Architectural-invariant gate: only `aionforge-store` (L0) may name selene-db
# crates, so a selene-db API change has a single point of contact (design 01 §2).
# Every other crate works through aionforge-store's typed surface.
#
# Checks member crate manifests (not the root workspace manifest, which is the
# central declaration point). Runs from repo root. macOS bash 3.x compatible.

set -euo pipefail

PATTERN='^[[:space:]]*selene-(core|graph|persist|gql|algorithms|testing)\b'
violations=0

while IFS= read -r f; do
  case "$f" in
    crates/aionforge-store/Cargo.toml) continue ;;
  esac
  if grep -nE "$PATTERN" "$f" >/dev/null 2>&1; then
    echo "FAIL: $f names a selene-db crate (only aionforge-store may):"
    grep -nE "$PATTERN" "$f" || true
    violations=$((violations + 1))
  fi
done < <(git ls-files 'crates/*/Cargo.toml' 2>/dev/null || true)

if [ "$violations" -gt 0 ]; then
  echo
  echo "Route selene-db access through aionforge-store's typed surface (design 01 §2)."
  exit 1
fi

echo "OK: only aionforge-store names selene-db crates."
