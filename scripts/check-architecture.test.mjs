import assert from "node:assert/strict";
import { mkdtemp, mkdir, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";
import { fileURLToPath } from "node:url";

const checker = fileURLToPath(new URL("./check-architecture.mjs", import.meta.url));

async function createWorkspace(manifests) {
  const root = await mkdtemp(join(tmpdir(), "kesharon-architecture-"));

  for (const [relativePath, content] of Object.entries(manifests)) {
    const target = join(root, relativePath);
    await mkdir(join(target, ".."), { recursive: true });
    await writeFile(target, content, "utf8");
  }

  return root;
}

function runChecker(root) {
  return spawnSync(process.execPath, [checker, root], {
    encoding: "utf8"
  });
}

test("accepts inward-only domain and application dependencies", async () => {
  const root = await createWorkspace({
    "crates/kesharon-domain/Cargo.toml": `
[package]
name = "kesharon-domain"
version = "0.0.0"
`,
    "crates/kesharon-application/Cargo.toml": `
[package]
name = "kesharon-application"
version = "0.0.0"

[dependencies]
kesharon-domain = { path = "../kesharon-domain" }
`
  });

  const result = runChecker(root);

  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /architecture boundaries verified/i);
});

test("rejects infrastructure dependencies from the domain", async () => {
  const root = await createWorkspace({
    "crates/kesharon-domain/Cargo.toml": `
[package]
name = "kesharon-domain"
version = "0.0.0"

[dependencies]
rusqlite = "0.37"
`
  });

  const result = runChecker(root);

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /kesharon-domain.*rusqlite/i);
});

test("rejects framework dependencies from the application layer", async () => {
  const root = await createWorkspace({
    "crates/kesharon-domain/Cargo.toml": `
[package]
name = "kesharon-domain"
version = "0.0.0"
`,
    "crates/kesharon-application/Cargo.toml": `
[package]
name = "kesharon-application"
version = "0.0.0"

[dependencies]
kesharon-domain = { path = "../kesharon-domain" }
tauri = "2"
`
  });

  const result = runChecker(root);

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /kesharon-application.*tauri/i);
});

test("rejects dependency-specific TOML tables in the domain", async () => {
  const root = await createWorkspace({
    "crates/kesharon-domain/Cargo.toml": `
[package]
name = "kesharon-domain"
version = "0.0.0"

[dependencies.rusqlite]
version = "0.37"
`
  });

  const result = runChecker(root);

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /kesharon-domain.*rusqlite/i);
});

test("rejects dotted dependency keys in the application layer", async () => {
  const root = await createWorkspace({
    "crates/kesharon-application/Cargo.toml": `
[package]
name = "kesharon-application"
version = "0.0.0"

[dependencies]
kesharon-domain = { path = "../kesharon-domain" }
tauri.workspace = true
`
  });

  const result = runChecker(root);

  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /kesharon-application.*tauri/i);
});
