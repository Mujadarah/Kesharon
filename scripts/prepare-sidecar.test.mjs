import assert from "node:assert/strict";
import path from "node:path";
import test from "node:test";

import {
  parseHostTriple,
  sidecarDestination
} from "./prepare-sidecar.mjs";

test("extracts the exact host triple from rustc verbose output", () => {
  const output = [
    "rustc 1.97.1",
    "binary: rustc",
    "host: x86_64-pc-windows-msvc",
    "release: 1.97.1"
  ].join("\n");

  assert.equal(parseHostTriple(output), "x86_64-pc-windows-msvc");
});

test("uses Tauri's target-suffixed external binary convention", () => {
  const destination = sidecarDestination(
    "C:\\workspace",
    "x86_64-pc-windows-msvc",
    "win32"
  );

  assert.equal(
    destination,
    path.join(
      "C:\\workspace",
      "apps",
      "desktop",
      "src-tauri",
      "binaries",
      "kesharon-daemon-x86_64-pc-windows-msvc.exe"
    )
  );
});

test("rejects rustc output without a host triple", () => {
  assert.throws(() => parseHostTriple("rustc 1.97.1"), /host triple/u);
});
