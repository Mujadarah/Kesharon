# ADR 0003: Tiered Execution Isolation

- Status: Accepted
- Date: 2026-07-29

## Decision

Use managed Git worktrees for repository mutation isolation. Provide restricted
native execution for trusted work and optional Docker or Podman execution for a
stronger isolation tier.

The product must describe native execution as weaker than container isolation.
Elevated host automation always requires explicit high-friction approval in v1.

## Consequences

- The base application has no mandatory container-runtime dependency.
- Policy and UI contracts include an isolation-level field.
- Security tests must cover path, symlink, junction, process, and network
  escapes independently on each operating system.
