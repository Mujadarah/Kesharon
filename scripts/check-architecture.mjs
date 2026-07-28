import { readFile, readdir } from "node:fs/promises";
import { basename, join, resolve } from "node:path";
import process from "node:process";

const workspaceRoot = resolve(process.argv[2] ?? process.cwd());
const cratesRoot = join(workspaceRoot, "crates");

async function findManifests(directory) {
  let entries;

  try {
    entries = await readdir(directory, { withFileTypes: true });
  } catch (error) {
    if (error.code === "ENOENT") {
      return [];
    }
    throw error;
  }

  const manifests = [];
  for (const entry of entries) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) {
      manifests.push(...await findManifests(path));
    } else if (entry.isFile() && entry.name === "Cargo.toml") {
      manifests.push(path);
    }
  }
  return manifests;
}

function normalDependencies(manifest) {
  const dependencies = new Set();
  let dependencySection = false;

  for (const rawLine of manifest.split(/\r?\n/)) {
    const line = rawLine.replace(/\s+#.*$/, "").trim();
    const section = line.match(/^\[(.+)]$/);

    if (section) {
      const name = section[1];
      dependencySection =
        name === "dependencies" ||
        name === "build-dependencies" ||
        /^target\..+\.dependencies$/.test(name) ||
        /^target\..+\.build-dependencies$/.test(name);
      continue;
    }

    if (!dependencySection || line.length === 0) {
      continue;
    }

    const dependency = line.match(/^([A-Za-z0-9_-]+)\s*=/);
    if (dependency) {
      dependencies.add(dependency[1]);
    }
  }

  return dependencies;
}

function validate(crateName, dependencies) {
  const violations = [];

  if (crateName === "kesharon-domain") {
    for (const dependency of dependencies) {
      violations.push(`${crateName} must use only the Rust standard library; found ${dependency}`);
    }
  }

  if (crateName === "kesharon-application") {
    for (const dependency of dependencies) {
      if (dependency !== "kesharon-domain") {
        violations.push(`${crateName} may depend only on kesharon-domain; found ${dependency}`);
      }
    }
  }

  return violations;
}

const manifests = await findManifests(cratesRoot);
const violations = [];

for (const manifestPath of manifests) {
  const crateName = basename(join(manifestPath, ".."));
  const manifest = await readFile(manifestPath, "utf8");
  violations.push(...validate(crateName, normalDependencies(manifest)));
}

if (violations.length > 0) {
  process.stderr.write(`${violations.join("\n")}\n`);
  process.exitCode = 1;
} else {
  process.stdout.write(`Architecture boundaries verified across ${manifests.length} crate manifests.\n`);
}
