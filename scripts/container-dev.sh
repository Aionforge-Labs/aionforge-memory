#!/usr/bin/env bash
# Build and run the Aionforge Memory OCI image with Apple's `container` runtime.

set -euo pipefail

IMAGE="${AIONFORGE_CONTAINER_IMAGE:-aionforge-memory:dev}"
NAME="${AIONFORGE_CONTAINER_NAME:-aionforge-memory}"
ARCH="${AIONFORGE_CONTAINER_ARCH:-arm64}"
HOST="${AIONFORGE_CONTAINER_HOST:-127.0.0.1}"
PORT="${AIONFORGE_CONTAINER_PORT:-3918}"
AGENT_ID="${AIONFORGE_AGENT_ID:-018f0cc0-40f3-7cc4-b8b4-9ca41f88d012}"
TOKEN="${AIONFORGE_MCP_TOKEN:-}"

usage() {
  cat <<'USAGE'
Usage: scripts/container-dev.sh <command>

Commands:
  build    Build the local OCI image with Apple's container builder
  run      Run a named container on 127.0.0.1:3918
  start    Start the named container
  stop     Stop the named container
  logs     Print logs for the named container
  status   List all containers
  delete   Delete the named container and its internal /data state

Environment:
  AIONFORGE_CONTAINER_IMAGE   Image tag to build/run (default: aionforge-memory:dev)
  AIONFORGE_CONTAINER_NAME    Container name (default: aionforge-memory)
  AIONFORGE_CONTAINER_ARCH    Build architecture (default: arm64)
  AIONFORGE_CONTAINER_HOST    Host bind address (default: 127.0.0.1)
  AIONFORGE_CONTAINER_PORT    Host port (default: 3918)
  AIONFORGE_AGENT_ID          Principal bound to the bearer token
  AIONFORGE_MCP_TOKEN         Bearer token; generated for local runs when unset
USAGE
}

require_container() {
  if ! command -v container >/dev/null 2>&1; then
    echo "Apple container CLI not found. Install it from https://github.com/apple/container/releases." >&2
    exit 127
  fi
}

ensure_system() {
  if ! container system status >/dev/null 2>&1; then
    container system start
  fi
}

generate_token() {
  if [ -n "$TOKEN" ]; then
    printf '%s' "$TOKEN"
    return
  fi
  if command -v openssl >/dev/null 2>&1; then
    openssl rand -hex 32
  else
    echo "AIONFORGE_MCP_TOKEN is required when openssl is unavailable." >&2
    exit 1
  fi
}

cmd="${1:-}"
case "$cmd" in
  build)
    require_container
    ensure_system
    container build --arch "$ARCH" --tag "$IMAGE" .
    ;;
  run)
    require_container
    ensure_system
    TOKEN="$(generate_token)"
    container run -d \
      --name "$NAME" \
      --env "AIONFORGE_AGENT_ID=$AGENT_ID" \
      --env "AIONFORGE_MCP_TOKEN=$TOKEN" \
      --publish "$HOST:$PORT:3918" \
      "$IMAGE"
    echo "MCP endpoint: http://$HOST:$PORT/mcp"
    echo "AIONFORGE_AGENT_ID=$AGENT_ID"
    echo "AIONFORGE_MCP_TOKEN=$TOKEN"
    ;;
  start)
    require_container
    ensure_system
    container start "$NAME"
    ;;
  stop)
    require_container
    container stop "$NAME"
    ;;
  logs)
    require_container
    container logs "$NAME"
    ;;
  status)
    require_container
    container list --all
    ;;
  delete)
    require_container
    container delete "$NAME"
    ;;
  ""|-h|--help|help)
    usage
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac
