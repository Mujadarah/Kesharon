# Kesharon engineering instructions

## Truth and evidence

- Never claim a milestone, test, build, package, or runtime works without fresh
  command output proving it.
- Distinguish source-checkout evidence from packaged and installed-runtime
  evidence.
- Record blockers and evidence gaps plainly.

## Architecture

- `kesharon-domain` may depend only on the Rust standard library.
- `kesharon-application` may depend on the domain and standard-library
  abstractions, never on infrastructure frameworks.
- Provider SDKs, SQLite, Git, subprocess, Tauri, and filesystem watcher types
  stay inside adapter or host crates.
- The React webview never receives generic shell or filesystem capabilities.
- Cross-boundary data uses the versioned protocol package.

## Delivery

- Use strict red-green-refactor TDD for behavior.
- Keep changes scoped to one milestone gate.
- Run architecture checks, tests, linting, and builds before completion claims.
- Do not add telemetry, cloud logging, or persistent secrets without an
  approved architecture decision.

## Git & Branch Hygiene

- Keep only the `main` branch permanently locally and on GitHub.
- Any new push to GitHub must be done via a topic branch and submitted as a Pull Request (PR) against `main`. Direct pushes to `main` are prohibited.
- Upon merging a PR into `main`, immediately delete the secondary branch both locally and on GitHub.
