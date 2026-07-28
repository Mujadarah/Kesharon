# ADR 0002: SQLite Storage and Write Discipline

- Status: Accepted
- Date: 2026-07-29

## Decision

Use bundled SQLite with FTS5 for durable state and initial graph storage.
Separate user-owned `state.db` from rebuildable `graph.db`.

Use a dedicated coalescing writer, content-hash suppression, bounded append
logs, explicit retention ceilings, WAL checkpoints, and no timestamp-only
rewrites. Do not introduce LMDB until a measured SQLite hot path requires it.

## Consequences

- Backup, export, recovery, and diagnostics remain understandable.
- Resource tests must detect WAL churn and repeated unchanged writes.
- Graph rebuilds cannot destroy session or approval data.
