# Initial Threat Model

## Assets

- Repository contents and uncommitted work
- Provider credentials and local secrets
- Session, approval, and task history
- Host filesystem, processes, network identity, and browser sessions
- Integrity of diffs, test results, resource evidence, and updates

## Trust boundaries

- Remote or local model output entering the application
- React webview to privileged Tauri host
- Tauri host to daemon IPC
- Daemon to repository, subprocess, network, credential, and container adapters
- Repository-controlled files, hooks, build scripts, and instructions
- Optional plugins, MCP/ACP servers, browser runtimes, and update artifacts

## Required controls

- Narrow Tauri capabilities and no generic shell command in the webview
- Same-user authenticated IPC with frame limits and protocol validation
- Path canonicalization with symlink and junction escape protection
- Declared tool effects evaluated by a deny-by-default policy engine
- Process-tree cancellation, timeouts, output limits, and network scoping
- Secret redaction before logs, tool output persistence, or support exports
- Signed update metadata, checksums, SBOMs, and provenance attestations
- Explicit distinction between native and container isolation

This document is the starting point for milestone M7. Each new trust boundary
requires an architecture decision and corresponding abuse-case tests.
