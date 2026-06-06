# Aionforge Memory

> **Status: early development (pre-alpha).** The architecture and build contract are
> set; the implementation is being built milestone by milestone. APIs, schema, and
> surfaces will change. Not yet released.

**Aionforge Memory** is a Rust-native **agentic-memory substrate**: long-term,
secure, temporally-correct recall for AI agents, built entirely on the native
surfaces of the embeddable graph engine [`selene-db`](https://github.com/jscott3201/selene-db).
It gives a single agent cross-session cohesion, gives a multi-agent system a shared
and provenance-bearing memory plane, and gives every agent a procedural-memory
(skill) layer.

It is exposed two ways:

- **As a Rust library** an embedding host links directly (the lowest-latency path).
- **As an optional [MCP](https://modelcontextprotocol.io) server** (Tools, Resources,
  Prompts) over stdio and streamable HTTP, for agentic harnesses such as Claude Code,
  Codex, Copilot, Cursor, and OpenCode.

A read-only [ratatui](https://ratatui.rs) operator TUI and a single CLI binary round
out the surface. It runs locally on macOS and Linux, or via Docker.

## Honest scope

Aionforge is **exemplar-based (retrieval) memory**, not weight-based (parametric)
memory. It improves recall quality, temporal correctness, multi-hop association,
cross-session cohesion, security, and token efficiency. It does **not** claim to make
a base model generalize or develop expertise by accumulating memories, and it ships no
weight-based learning. It runs no inference itself — it calls an external
OpenAI-compatible endpoint for embeddings (and, optionally, extraction/rerank).

## Design pillars

- **Engine-native only** — all storage, indexing, hybrid search (BM25 + dense vectors
  + graph), and graph algorithms go through selene-db; no bolt-on search engine or
  external vector index.
- **Bi-temporal and non-lossy** — facts carry event-time and transaction-time windows;
  updates supersede rather than destroy; hard erasure is a separate, auditable path.
- **Two-path writes** — fast millisecond capture plus slow asynchronous consolidation.
- **Query-class-conditional hybrid retrieval** — rank-fused signals with graph
  expansion applied only where it helps precision.
- **Security as a v1 requirement** — provenance, per-writer trust, namespace
  authorization, quarantine-on-contradiction, structural untrusted-data tagging, and a
  red-team acceptance suite.
- **Determinism** — identical inputs and graph state yield identical retrieval
  ordering; derived state is rebuildable from the primary graph.

## Building

Requires the Rust toolchain pinned in `rust-toolchain.toml` (1.95.0, edition 2024).

Aionforge depends on `selene-db` as a path dependency. Check it out as a **sibling**
directory:

```text
parent/
├── aionforge-memory/   # this repository
└── selene-db/          # https://github.com/jscott3201/selene-db (development branch)
```

```bash
git clone https://github.com/jscott3201/selene-db.git ../selene-db
cargo build --workspace --locked
cargo nextest run --workspace --locked --all-features
```

Install the shared git hooks once per clone:

```bash
bash scripts/install-hooks.sh
```

## License

Dual-licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option. Unless you explicitly state otherwise, any contribution
intentionally submitted for inclusion in this work, as defined in the Apache-2.0
license, shall be dual-licensed as above, without any additional terms or conditions.
