#!/usr/bin/env node

// Builds a self-contained distributable for the host platform:
//   Linux   -> AppImage (webkit2gtk/GTK bundled by Tauri's linuxdeploy; only glibc comes from the
//              host, so build on the oldest distro you want to support).
//   Windows -> portable folder + .zip: the exe with llama/ and workshop/ beside it, which is where
//              Tauri's resource_dir() points for an unbundled exe. Set WEBVIEW2_FIXED_RUNTIME to an
//              extracted "WebView2 Fixed Version Runtime" folder to ship WebView2 too; without it the
//              app uses the system WebView2 that Windows 10/11 already ship.
// Neither binary can be cross-built: llama-server and workerd are fetched for the host platform.

import { execFileSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, symlinkSync } from "node:fs";
import { tmpdir } from "node:os";
import { delimiter, dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const DESKTOP = dirname(dirname(fileURLToPath(import.meta.url)));
const TAURI = join(DESKTOP, "src-tauri");
const RESOURCES = join(TAURI, "resources");
const windows = process.platform === "win32";

const run = (cmd: string, args: string[], env: Record<string, string> = {}) =>
  execFileSync(cmd, args, { cwd: DESKTOP, stdio: "inherit", shell: windows, env: { ...process.env, ...env } });

if (!existsSync(join(RESOURCES, "llama", windows ? "llama-server.exe" : "llama-server"))) {
  run("pnpm", ["run", "fetch-llama"]);
}
run("pnpm", ["run", "build-runtime"]);

if (process.platform === "linux") {
  // linuxdeploy is itself an AppImage; extract-and-run avoids needing FUSE (WSL, containers, CI).
  // A Homebrew pkg-config earlier on PATH can't see the distro's GTK/webkit2gtk, and linuxdeploy's
  // gtk plugin calls `pkgconf` or `pkg-config` from PATH (ignoring PKG_CONFIG), so shadow both with
  // the system one.
  const env: Record<string, string> = { APPIMAGE_EXTRACT_AND_RUN: "1" };
  if (existsSync("/usr/bin/pkg-config")) {
    const shim = mkdtempSync(join(tmpdir(), "pkg-config-"));
    for (const name of ["pkg-config", "pkgconf"]) symlinkSync("/usr/bin/pkg-config", join(shim, name));
    env.PATH = `${shim}${delimiter}${process.env.PATH}`;
  }
  run("pnpm", ["exec", "tauri", "build", "--bundles", "appimage"], env);
  const out = join(TAURI, "target", "release", "bundle", "appimage");
  console.log(`\nAppImage: ${join(out, readdirSync(out).find((f) => f.endsWith(".AppImage"))!)}`);
} else if (windows) {
  const webview2 = process.env.WEBVIEW2_FIXED_RUNTIME;
  const args = ["exec", "tauri", "build", "--no-bundle"];
  // Tauri points WEBVIEW2_BROWSER_EXECUTABLE_FOLDER at <exe dir>/<path> at startup.
  if (webview2) args.push("--config", JSON.stringify({ bundle: { windows: { webviewInstallMode: { type: "fixedRuntime", path: "webview2" } } } }));
  run("pnpm", args);

  const release = join(TAURI, "target", "release");
  const dir = join(release, "portable", "Gadgets Desktop");
  rmSync(dirname(dir), { recursive: true, force: true });
  mkdirSync(dir, { recursive: true });
  cpSync(join(release, "gadgets-desktop.exe"), join(dir, "Gadgets Desktop.exe"));
  cpSync(join(RESOURCES, "llama"), join(dir, "llama"), { recursive: true });
  cpSync(join(RESOURCES, "workshop"), join(dir, "workshop"), { recursive: true });
  if (webview2) cpSync(webview2, join(dir, "webview2"), { recursive: true });
  else console.warn("WEBVIEW2_FIXED_RUNTIME not set: the portable build relies on the system WebView2.");

  // Windows 10+ ships bsdtar as `tar`; -a picks the format from the .zip extension.
  const zip = join(release, "portable", "Gadgets-Desktop-portable-x64.zip");
  execFileSync("tar", ["-a", "-cf", zip, "-C", dirname(dir), "Gadgets Desktop"], { stdio: "inherit" });
  console.log(`\nPortable: ${dir}\nZip: ${zip}`);
} else {
  console.error(`No package target for ${process.platform}`);
  process.exit(1);
}
