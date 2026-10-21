# Desktop phase 2: decision log

Goal: the desktop app runs the Workshop with no repo, pnpm or system Node. One row per decision,
newest last. Evidence is reproducible from `scripts/build-runtime.ts` unless noted.

## Runtime shape

| Option | What ships | Evidence | Verdict |
| --- | --- | --- | --- |
| (a) Bundled Node + Miniflare launcher | node (124 MB on this machine) + miniflare (26 MB) + workerd (146 MB) + bundles + a launcher script | `build-runtime.ts` runs exactly this at build time, and its smoke test (`GET /` through Miniflare) passes. So (a) works. | Rejected: about 150 MB more on disk, a second runtime to pin, checksum and update per OS, and an extra process between the app and workerd, all to run code whose only job is to produce a config. |
| (b) Raw `workerd serve` with a hand-written capnp config | workerd + config | workerd has no built-in KV, R2 or Cache: in the captured config those bindings are Miniflare's own JS workers (`miniflare-KVNamespaceObject`, `miniflare-R2BucketObject`, `miniflare-CacheObject`) backed by Durable Objects on disk services. Hand-writing that means reimplementing Miniflare's simulators and re-deriving wrangler's binding translation (DO migrations, `ctx.exports`, worker loaders, assets). | Rejected as stated. It is right about the shipping shape, though, and (c) gets that shape without hand-writing anything. |
| (c) Capture Miniflare's generated workerd config at build time, run raw workerd at runtime | workerd (146 MB, 38 MB gzipped) + `config.bin` (15 MB, all modules embedded) + frontend (3.3 MB) + `runtime.json` | Built config copied outside the repo and run with `env -i PATH=/usr/bin:/bin`: `/` serves the SPA, `/api` answers, `/gatekeeper/context` answers, sign-up works, and the login works again after a workerd restart with the same data dir. | **Chosen.** Same simulators and binding translation as `wrangler dev`, with no Node at runtime. |

## How (c) is made portable

| Decision | Why | Evidence |
| --- | --- | --- |
| Capture via `Runtime.prototype.updateConfig` (Miniflare exports `Runtime`) | Cross-platform, no fake workerd binary or shell shim. | The capture is the exact buffer Miniflare pipes to workerd. |
| Miniflare resolved through wrangler (`createRequire(wrangler)`), workerd through Miniflare | The captured config and the shipped workerd always come from the same release wrangler uses, so no extra pinned dependency can drift. | `miniflare@5.20260831.0-alpha`, `workerd@1.20260831.1`. |
| Miniflare 5 option names: `convertV4MiniflareOptions`, `resourcePersistencePath`, `resourceTmpPath` | wrangler's `unstable_getMiniflareWorkerOptions` still emits v4 worker options; Miniflare 5 rejects them unconverted, and v5 ignores `defaultPersistRoot`. | First runs failed with `config: undefined` validation errors, then with no state dir created. |
| Persistence and temp paths are relative (`state/`, `tmp/`), and `TMPDIR=tmp` while Miniflare runs | workerd resolves a relative disk path against its working directory, so the app picks the state location by `current_dir`. Miniflare puts some storage (email) under `os.tmpdir()` regardless of `resourceTmpPath`. | First raw run failed `Directory named "email:store-disk" not found: /tmp/miniflare-…/email-store`. |
| The build fails if the config still names a build-machine path (except `//# sourceURL` comments and the one assets path) | That bug class (above) only shows on another machine. The gate made it visible on this one. | The gate caught the email-store path; it also showed `/tmp` alone is too broad a root (bundled code contains `/tmp/artifacts/…`), so it checks the scratch dir, `<tmp>/miniflare-` and the repo root. |
| Assets: copy the frontend to `workshop/assets`, override `assets:storage` with `--directory-path` | Miniflare's assets disk service takes an absolute path; it cannot also be relative to the state dir. | `runtime.json` names the service; the build checks it exists. |
| `runtime.json` lists every directory the config's disk services need | workerd refuses to start when a disk directory is missing. | `Directory named "cache:storage" not found: state/cache`. |
| `--external-addr=loopback=127.0.0.1:9` | The config declares Miniflare's Node loopback service and workerd requires an address; nothing the Workshop needs calls it, and port 9 refuses connections. | Sign-up, login, gatekeepers and chat (below) ran with it. |

## Product decisions

| Decision | Why |
| --- | --- |
| Port 8797, fixed, bound to 127.0.0.1 | Not 8787, so a dev `run-local` can run alongside. Fixed, because the Workshop keeps its session in the origin's localStorage: a new port each launch would log the user out. The built frontend derives the API host from `window.location`, so the only port-dependent config is `PUBLIC_BASE_URL` and the gatekeepers' `BASE_URL`, baked by the build script. |
| Workers: router, workshop-backend, gatekeeper-context, -scheduler, -mcp, -homeassistant | The gatekeepers `manifest-lib.ts` marks `NO_DEFAULT_CRED_INPUTS`. mcp-portal is out (useless until an admin configures a portal URL), email is out (needs Email Routing), the OAuth ones are out (need app secrets). |
| Production router with its `ASSETS` binding, not run-local's backend-serves-assets layout | Mirrors a real deployment; no dev-router special case. |
| Password-login frontend (`build:assets`), `ADMINS = ["admin"]` | Same as run-local: password auth works with no identity provider, and the `admin` account reaches `/admin`. |
| Browser Run binding removed | Miniflare emulates it by launching Chrome from Node. The binding is optional; browser-rendered exports fail with "Gadget export is not configured for this deployment" (`overseer.ts`). Workers AI is never bound; chat uses a configured provider such as llama.cpp. |
| Start workerd at app launch; Abrir Workshop waits (60 s) for `GET /` | Usually ready before the click. A failure surfaces the tail of `<app data>/workshop/workerd.log` (e.g. port in use). |
| Phase-1 `pnpm run-local` path deleted, not kept behind an env var | Two launch paths to keep working; a developer who wants run-local can open `localhost:8787` in a browser. |
| No kernel changes | Everything is configuration of existing bindings; `workshop-backend` and `workshop-shared` are untouched apart from the cherry-picked llamacpp provider commits. |
| workerd gets `PR_SET_PDEATHSIG(SIGKILL)` on Linux | Driving the app with WebDriver ends it with a hard kill, which skips `RunEvent::Exit`; the orphaned workerd kept 8797, and the next launch's workerd failed with `Address already in use` while the window silently opened onto the orphan. macOS and Windows still orphan on a hard kill (noted in `main.rs`). |

## Verification

All on WSL2 Ubuntu 24.04, the user's Node (`~/.nvm`) excluded from `PATH`.

| Check | Result |
| --- | --- |
| Runtime copied outside the repo, run with `env -i PATH=/usr/bin:/bin` | VERIFIED. `GET /` 200 (SPA), `/api` 400 "only accepts POST or WebSocket", `/gatekeeper/context` 200, SPA fallback 200. |
| Sign-up, then workerd restart, then password login (Playwright, fresh browser) | VERIFIED. Both land on the onboarding wizard as `admin`; 23 SQLite files under `state/do` after the restart. |
| Agent chat round-trip with the llamacpp provider (Qwen3-4B on llama-server 8089, through the Workshop UI) | VERIFIED. Model added via the llama.cpp entry in Add Model; "Reply with exactly the word PONG" answered `PONG` (two runs: about 40 min cold, CPU prefill of an 8k-token prompt at load average ~19; 169 s warm). workerd logged `agent.run.finished outcome: 'ok'`, no loopback or missing-binding errors besides the expected "Flagship binding missing; using default values" warning. |
| `tauri build --bundles deb` | VERIFIED. 72.5 MiB `.deb`, 249 MB installed (workerd 146 MB, config 15 MB, frontend 3.3 MB, llama.cpp 44 MB); `workerd` keeps its executable bit. |
| The `.deb`'s app, extracted, driven with WebKitWebDriver 2.52.6 (`apt-get download webkit2gtk-driver`), `PATH=/usr/bin:/bin` | VERIFIED. workerd answers 1.0 s after launch; Abrir Workshop opens the Workshop window on `http://127.0.0.1:8797/` in about 1.5 s; sign-up works there; on the next launch the window opens still signed in, and after clearing its storage the password login works; 8797 is free after each exit. |
| Not verified | macOS and Windows builds (the runtime is per-platform: `build-runtime` must run on each target OS); Windows console-window behaviour for workerd; one early WebDriver run where the runtime was not listening and `workerd.log` was empty, which did not recur across five later launches. |
