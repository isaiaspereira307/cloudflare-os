#!/usr/bin/env node

// Builds the self-contained Workshop runtime into src-tauri/resources/workshop/: the `workerd`
// binary plus one binary workerd config holding every worker the desktop app runs. Nothing at
// runtime needs Node, pnpm or this repo.
//
// The config is the one Miniflare would hand workerd: each worker is bundled exactly as the release
// does (`wrangler deploy --dry-run`), wrangler translates its config into Miniflare options, and
// Miniflare assembles the workerd config, which carries its KV/R2/cache simulators and embeds every
// module. Miniflare is started once here only to capture that config (and to smoke-test it), so
// the app runs `workerd serve` directly. Persistence paths are relative, so the app chooses where
// state lives by choosing workerd's working directory. See docs/phase2-decisions.md.
//
// Usage: node scripts/build-runtime.ts

import { execFile } from "node:child_process";
import { copyFileSync, cpSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import { collectModules, type ModuleType } from "../../../scripts/release/hash-lib.ts";

const DESKTOP_DIR = dirname(dirname(fileURLToPath(import.meta.url)));
const ROOT = join(DESKTOP_DIR, "..", "..");
const PACKAGES_DIR = join(ROOT, "packages");
const OUT = join(DESKTOP_DIR, "src-tauri", "resources", "workshop");
const FRONTEND_DIST = join(PACKAGES_DIR, "workshop-frontend", "dist");
// The disk service Miniflare serves static assets from. Its path is absolute, so the app points it
// at its own copy of the frontend with `workerd --directory-path`.
const ASSETS_SERVICE = "assets:storage";

// Not 8787, so a dev `pnpm run-local` can run alongside. Fixed rather than picked at launch: the
// Workshop keeps its session in the origin's localStorage, so a new port would mean a new login.
const PORT = 8797;
const ORIGIN = `http://127.0.0.1:${PORT}`;

// Gatekeepers that need no third-party OAuth app credentials, so they work out of the box.
// gatekeeper-mcp-portal is left out too: it is useless until an admin configures a portal URL.
const GATEKEEPERS = [
  "gatekeeper-context",
  "gatekeeper-scheduler",
  "gatekeeper-mcp",
  "gatekeeper-homeassistant",
];

// Relative, so workerd resolves them against its working directory: the app's data dir.
const STATE_DIR = "state";
const TMP_DIR = "tmp";

const MINIFLARE_MODULE_TYPE: Record<ModuleType, string> = {
  esm: "ESModule",
  text: "Text",
  wasm: "CompiledWasm",
  data: "Data",
};

const run = promisify(execFile);
const requireFromRoot = createRequire(join(ROOT, "package.json"));
const wrangler = await import(requireFromRoot.resolve("wrangler"));
// The Miniflare (and through it the workerd) wrangler itself runs, so the captured config always
// matches the runtime that is shipped with it.
const requireFromWrangler = createRequire(requireFromRoot.resolve("wrangler"));
const { Miniflare, Runtime, convertV4MiniflareOptions } = await import(requireFromWrangler.resolve("miniflare"));
const workerdBinary: string = createRequire(requireFromWrangler.resolve("miniflare"))("workerd").default;

const binding = (name: string) => `GATEKEEPER_${name.slice("gatekeeper-".length).toUpperCase()}`;

async function pnpm(args: string[], cwd = ROOT): Promise<void> {
  console.log(`> pnpm ${args.join(" ")}`);
  await run("pnpm", args, { cwd, maxBuffer: 64 * 1024 * 1024 });
}

// The generated sources every bundle needs (bundled blueprints, gatekeeper UIs) and the frontend
// the router serves. The password-login frontend, as run-local builds it.
await pnpm(["exec", "vp", "run", "--cache", "@gadgets/typed-storage#build"]);
await pnpm(["exec", "vp", "run", "--cache", "@gadgets/workshop-frontend#build:assets"]);
// By package name, which is not always `@gadgets/<dir>` (`@gadgets/mcp-gatekeeper`), and a
// `vp run` naming no package exits 0 having built nothing.
for (const dir of ["workshop-backend", ...GATEKEEPERS]) {
  const { name } = JSON.parse(readFileSync(join(PACKAGES_DIR, dir, "package.json"), "utf8"));
  await pnpm(["exec", "vp", "run", "--cache", `${name}#build`]);
}

/** A worker's normalized wrangler config, adjusted for the desktop. */
function desktopConfig(name: string) {
  const config = wrangler.unstable_readConfig({ config: join(PACKAGES_DIR, name, "wrangler.jsonc") });
  if (name === "router") {
    config.services.push(...GATEKEEPERS.map(gk => ({ binding: binding(gk), service: gk })));
  } else if (name === "workshop-backend") {
    // As run-local: sign up as "admin" to reach the admin panel.
    config.vars.ADMINS = ["admin"];
    config.vars.PUBLIC_BASE_URL = ORIGIN;
    config.services.push(...GATEKEEPERS.map(gk => ({
      binding: binding(gk),
      service: gk,
      entrypoint: "GatekeeperVendor",
      ...(gk === "gatekeeper-context" ? { props: { sharingDomain: "desktop" } } : {}),
    })));
    // Miniflare emulates Browser Run by launching a local Chrome from Node, which the desktop does
    // not ship. The binding is optional: browser-rendered Gadget exports (PDF) then fail with "Gadget
    // export is not configured for this deployment" (overseer.ts).
    config.browser = undefined;
  } else {
    config.vars.BASE_URL = `${ORIGIN}/gatekeeper/${name.slice("gatekeeper-".length)}`;
  }
  return config;
}

const scratch = mkdtempSync(join(tmpdir(), "gadgets-desktop-runtime-"));
try {
  const names = ["router", "workshop-backend", ...GATEKEEPERS];
  const workers = await Promise.all(names.map(async name => {
    const outDir = join(scratch, "bundles", name);
    await pnpm(["exec", "wrangler", "deploy", "--dry-run", "--outdir", outDir], join(PACKAGES_DIR, name));
    const { mainModule, modules } = collectModules(outDir);
    const { workerOptions } = wrangler.unstable_getMiniflareWorkerOptions(desktopConfig(name));
    return {
      ...workerOptions,
      name,
      modulesRoot: outDir,
      // Miniflare treats the first module as the entry point.
      modules: modules.toSorted((a, b) => Number(b.name === mainModule) - Number(a.name === mainModule))
          .map(m => ({ type: MINIFLARE_MODULE_TYPE[m.type], path: join(outDir, m.name), contents: m.bytes })),
    };
  }));

  let config: Buffer | undefined;
  const updateConfig = Runtime.prototype.updateConfig;
  Runtime.prototype.updateConfig = function (buffer: Buffer, ...rest: unknown[]) {
    config = buffer;
    return updateConfig.call(this, buffer, ...rest);
  };

  // Started from a directory of its own so the relative directories Miniflare creates land there,
  // which is also how the list of directories the app has to create is found.
  const cwd = join(scratch, "cwd");
  mkdirSync(cwd);
  process.chdir(cwd);
  const buildTmp = tmpdir();
  // Miniflare keeps some services' storage under os.tmpdir() (email, for one), which honours TMPDIR.
  process.env.TMPDIR = TMP_DIR;
  const mf = new Miniflare(convertV4MiniflareOptions({
    host: "127.0.0.1",
    port: PORT,
    resourcePersistencePath: STATE_DIR,
    resourceTmpPath: TMP_DIR,
    workers,
  }));
  let dirs: string[];
  try {
    const response = await mf.dispatchFetch(`${ORIGIN}/`);
    const body = await response.text();
    if (!response.ok || !body.includes("<div id=\"root\"")) {
      throw new Error(`smoke test failed: GET / answered ${response.status}`);
    }
    // Before dispose(), which deletes the temp directory.
    // .wrangler is wrangler's own cache, which nothing in the config refers to.
    dirs = listDirs(cwd).map(dir => dir.split(sep).join("/")).filter(dir => !dir.startsWith(".wrangler"));
  } finally {
    await mf.dispose();
  }
  if (!config) throw new Error("Miniflare never produced a workerd config");
  // An absolute path into the build machine would break the runtime once it is installed elsewhere.
  // The one expected is the frontend directory, which the app overrides (ASSETS_SERVICE).
  const leaked = buildMachinePaths(config, [scratch, join(buildTmp, "miniflare-"), ROOT]);
  if (!config.includes(ASSETS_SERVICE) || !leaked.delete(FRONTEND_DIST) || leaked.size) {
    throw new Error(`unexpected build-machine paths in the workerd config:\n  ${[...leaked].join("\n  ")}`);
  }

  rmSync(OUT, { recursive: true, force: true });
  mkdirSync(OUT, { recursive: true });
  writeFileSync(join(OUT, "config.bin"), config);
  const binary = process.platform === "win32" ? "workerd.exe" : "workerd";
  copyFileSync(workerdBinary, join(OUT, binary));
  cpSync(FRONTEND_DIST, join(OUT, "assets"), { recursive: true });
  writeFileSync(join(OUT, "runtime.json"), JSON.stringify({
    port: PORT,
    assetsService: ASSETS_SERVICE,
    dirs,
  }, null, 2) + "\n");
  console.log(`Workshop runtime (${names.join(", ")}) -> ${OUT}`);
} finally {
  process.chdir(ROOT);
  rmSync(scratch, { recursive: true, force: true });
}

function listDirs(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true, recursive: true })
      .filter(entry => entry.isDirectory())
      .map(entry => relative(dir, join(entry.parentPath, entry.name)));
}

// Every path under `roots` the config names, except in `//# sourceURL` comments, which only label
// stack traces.
function buildMachinePaths(config: Buffer, roots: string[]): Set<string> {
  const text = config.toString("latin1");
  const found = new Set<string>();
  for (const root of roots) {
    for (let at = text.indexOf(root); at !== -1; at = text.indexOf(root, at + root.length)) {
      if (!text.startsWith("sourceURL=file://", at - "sourceURL=file://".length)) {
        found.add(text.slice(at).split(/[^\x20-\x7e]/)[0]);
      }
    }
  }
  return found;
}
