import { copyFileSync, mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { spawnSync } from "node:child_process";

export function parseHostTriple(output) {
  const match = /^host:\s+(\S+)$/mu.exec(output);
  if (!match) {
    throw new Error("rustc output did not contain a host triple");
  }
  return match[1];
}

export function sidecarDestination(root, hostTriple, platform = process.platform) {
  const suffix = platform === "win32" ? ".exe" : "";
  return path.join(
    root,
    "apps",
    "desktop",
    "src-tauri",
    "binaries",
    `kesharon-daemon-${hostTriple}${suffix}`
  );
}

export function cargoBuildArguments(profile, root = ".") {
  if (profile !== "debug" && profile !== "release") {
    throw new Error(`unknown sidecar build profile: ${profile}`);
  }
  const arguments_ = ["build", "-p", "kesharon-daemon"];
  if (profile === "release") {
    arguments_.push("--release");
  }
  arguments_.push(
    "--target-dir",
    path.join(root, "target", `sidecar-${profile}`)
  );
  return arguments_;
}

export function sidecarSource(root, profile, platform = process.platform) {
  cargoBuildArguments(profile, root);
  const suffix = platform === "win32" ? ".exe" : "";
  return path.join(
    root,
    "target",
    `sidecar-${profile}`,
    profile,
    `kesharon-daemon${suffix}`
  );
}

function run(command, args, root) {
  const result = spawnSync(command, args, {
    cwd: root,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "inherit"]
  });
  if (result.status !== 0) {
    throw new Error(`${command} ${args.join(" ")} failed`);
  }
  return result.stdout;
}

export function prepareSidecar(root, profile = "debug") {
  const cargo = process.env.CARGO ?? "cargo";
  const rustc = process.env.RUSTC ?? "rustc";
  const hostTriple = parseHostTriple(run(rustc, ["-vV"], root));
  run(cargo, cargoBuildArguments(profile, root), root);

  const source = sidecarSource(root, profile);
  const destination = sidecarDestination(root, hostTriple);
  mkdirSync(path.dirname(destination), { recursive: true });
  copyFileSync(source, destination);
  return destination;
}

const currentFile = fileURLToPath(import.meta.url);
if (
  process.argv[1] &&
  pathToFileURL(path.resolve(process.argv[1])).href === pathToFileURL(currentFile).href
) {
  const repositoryRoot = path.resolve(path.dirname(currentFile), "..");
  const profile = process.argv.includes("--release") ? "release" : "debug";
  const destination = prepareSidecar(repositoryRoot, profile);
  process.stdout.write(`${destination}\n`);
}
