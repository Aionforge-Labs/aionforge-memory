#!/usr/bin/env bash
# One-time local setup: point git at the tracked .githooks/ directory.
# Run once per clone:  bash scripts/install-hooks.sh
#
# .githooks/ is version-controlled (unlike .git/hooks/), so the team shares the
# same gates. Mirrors the CI split:
#   pre-commit -> cargo fmt --check + file-size + no-secret + gql-binding  (fast)
#   pre-push   -> cargo clippy -D warnings (fast; the full nextest + doctest
#                 suite runs at the development->main release gate and inside
#                 agent workflows, not on every push)
#
# Escape hatches: `git commit/push --no-verify` (once) or
# `export AIONFORGE_SKIP_HOOKS=1` (whole shell session).
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"

git config core.hooksPath .githooks
chmod +x .githooks/pre-commit .githooks/pre-push 2>/dev/null || true

echo "core.hooksPath -> .githooks"
echo "  pre-commit: cargo fmt --check + file-size cap + no-secret scan + GQL binding gate"
echo "  pre-push:   cargo clippy -D warnings (fast; full suite at release gate)"
echo "Skip once: --no-verify   |   skip session: export AIONFORGE_SKIP_HOOKS=1"
