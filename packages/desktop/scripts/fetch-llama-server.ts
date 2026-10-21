#!/usr/bin/env node

// Downloads the pinned llama.cpp release for the host platform and extracts it into
// src-tauri/resources/llama/, which Tauri bundles as a resource directory. The release ships
// llama-server next to its shared libraries, so the whole directory is kept together.

import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdirSync, mkdtempSync, readdirSync, renameSync, rmSync, statSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const TAG = "b11457";

const ASSETS: Record<string, { name: string; sha256: string }> = {
  "linux-x64": {
    name: `llama-${TAG}-bin-ubuntu-x64.tar.gz`,
    sha256: "210eaa41a16e0d24fa44071cb62f95702ef903c84d86506d6482ac919bcee078",
  },
  "darwin-arm64": {
    name: `llama-${TAG}-bin-macos-arm64.tar.gz`,
    sha256: "e234070cbde0c8b0d30f79fa08ff8246abe22c72acb752619349b8d9c46f7839",
  },
  "win32-x64": {
    name: `llama-${TAG}-bin-win-cpu-x64.zip`,
    sha256: "ed20ed40e10c04d0853d8acf8700f0e6c1bb8212f0cc70c3a0da13dcbbc1b802",
  },
};

const platform = `${process.platform}-${process.arch}`;
const asset = ASSETS[platform];
if (!asset) {
  console.error(`No pinned llama.cpp build for ${platform}. Supported: ${Object.keys(ASSETS).join(", ")}`);
  process.exit(1);
}

const target = join(dirname(dirname(fileURLToPath(import.meta.url))), "src-tauri", "resources", "llama");
const binary = process.platform === "win32" ? "llama-server.exe" : "llama-server";
const url = `https://github.com/ggml-org/llama.cpp/releases/download/${TAG}/${asset.name}`;

console.log(`Downloading ${url}`);
const response = await fetch(url);
if (!response.ok) {
  console.error(`Download failed: HTTP ${response.status}`);
  process.exit(1);
}
const archive = Buffer.from(await response.arrayBuffer());
const digest = createHash("sha256").update(archive).digest("hex");
if (digest !== asset.sha256) {
  console.error(`Checksum mismatch for ${asset.name}: expected ${asset.sha256}, got ${digest}`);
  process.exit(1);
}

// Next to the target so the final rename never crosses filesystems.
mkdirSync(dirname(target), { recursive: true });
const work = mkdtempSync(join(dirname(target), ".fetch-"));
try {
  const archivePath = join(work, asset.name);
  writeFileSync(archivePath, archive);
  const extracted = join(work, "out");
  mkdirSync(extracted);
  // Windows 10+ ships bsdtar as `tar`, which also reads the .zip.
  execFileSync("tar", ["-xf", archivePath, "-C", extracted], { stdio: "inherit" });
  const binDir = findDirContaining(extracted, binary);
  if (!binDir) throw new Error(`${binary} not found in ${asset.name}`);
  rmSync(target, { recursive: true, force: true });
  renameSync(binDir, target);
  console.log(`Installed llama.cpp ${TAG} into ${target}`);
} finally {
  rmSync(work, { recursive: true, force: true });
}

function findDirContaining(dir: string, file: string): string | undefined {
  const entries = readdirSync(dir);
  if (entries.includes(file)) return dir;
  for (const entry of entries) {
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) {
      const found = findDirContaining(path, file);
      if (found) return found;
    }
  }
  return undefined;
}
