#!/usr/bin/env bash
# Mandatory-parameter-binding gate (spec 00 §3, 01 §5, 03 §8, 07).
#
# Caller-supplied values must NEVER be string-interpolated into a GQL statement;
# they are bound as `$name` parameters via the storage layer's bind API. This is
# a heuristic tripwire, not a parser: it flags the common footgun of building a
# GQL string with `format!`/`write!`/concatenation. Real safety is the binding
# architecture + the M0.T03 adversarial-input round-trip test.
#
# Escape hatch: append `// gql-ident-ok` to a line that legitimately interpolates
# a TRUSTED STATIC IDENTIFIER (a label or property name — GQL cannot bind those
# as parameters). Values must always be bound, never excepted.
#
# Runs from repo root. macOS bash 3.x compatible.

set -euo pipefail

GQL_KW='MATCH|MERGE|CREATE|RETURN|WHERE|DELETE|DETACH|REMOVE|CALL|YIELD|INSERT|UNWIND'
ALLOW='gql-ident-ok'
violations=0

check() {
  local pattern="$1"
  local label="$2"
  local hits
  hits=$(git grep -nE "$pattern" -- '*.rs' 2>/dev/null | grep -vE "$ALLOW" || true)
  if [ -n "$hits" ]; then
    echo "FAIL: possible GQL value interpolation ($label):"
    echo "$hits"
    echo
    violations=$((violations + 1))
  fi
}

# format!/write!-family building a GQL statement with a `{...}` placeholder.
check "(format!|write!|writeln!|format_args!)[[:space:]]*\\([[:space:]]*r?\".*($GQL_KW).*\\{" \
  "format!/write! into a GQL statement"

# String-literal concatenation into a GQL statement.
check "\".*($GQL_KW).*\"[[:space:]]*\\+" \
  "string concatenation into a GQL statement"

if [ "$violations" -gt 0 ]; then
  echo "Bind caller values as GQL parameters instead of interpolating them."
  echo "If the interpolated token is a trusted static identifier, add // gql-ident-ok."
  exit 1
fi

echo "OK: no GQL value-interpolation patterns found."
