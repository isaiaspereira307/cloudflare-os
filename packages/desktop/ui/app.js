const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const STATUS_TEXT = { stopped: "parado", starting: "iniciando…", ready: "pronto", failed: "erro" };
const MAX_LOGS = 40;

const ui = {
  catalog: [],
  downloaded: [],
  selected: null,
  progress: new Map(),
  llama: { status: "stopped" },
  logs: [],
};

const $ = (id) => document.getElementById(id);

function el(tag, props, ...children) {
  const node = Object.assign(document.createElement(tag), props);
  node.append(...children);
  return node;
}

function formatBytes(bytes) {
  return bytes >= 1024 ** 3 ? `${(bytes / 1024 ** 3).toFixed(2)} GB` : `${(bytes / 1024 ** 2).toFixed(1)} MB`;
}

function filenameFromUrl(url) {
  return decodeURIComponent(new URL(url).pathname.split("/").pop() ?? "");
}

function showError(error) {
  $("error").textContent = error ? String(error) : "";
}

async function refreshDownloaded() {
  ui.downloaded = await invoke("list_downloaded");
  if (!ui.downloaded.some((m) => m.filename === ui.selected)) ui.selected = ui.downloaded[0]?.filename ?? null;
  render();
}

async function download(url) {
  showError();
  const filename = filenameFromUrl(url);
  ui.progress.set(filename, { received: 0, total: null });
  render();
  try {
    await invoke("download_model", { url, filename });
  } catch (error) {
    showError(error);
  } finally {
    ui.progress.delete(filename);
    await refreshDownloaded();
  }
}

function progressFor(filename) {
  const progress = ui.progress.get(filename);
  if (!progress) return [];
  const bar = el("progress", { max: progress.total ?? 0, value: progress.received });
  if (!progress.total) bar.removeAttribute("value");
  return [bar, el("span", { textContent: formatBytes(progress.received) })];
}

function render() {
  const have = new Set(ui.downloaded.map((m) => m.filename));
  $("catalog").replaceChildren(...ui.catalog.map((model) => {
    const filename = filenameFromUrl(model.url);
    const busy = ui.progress.has(filename);
    return el("li", {},
      el("span", { className: "name", textContent: `${model.name} — ${formatBytes(model.size)}` }),
      have.has(filename)
        ? el("span", { textContent: "baixado" })
        : el("button", { type: "button", textContent: busy ? "Baixando…" : "Baixar", disabled: busy, onclick: () => download(model.url) }),
      ...progressFor(filename));
  }));

  const custom = [...ui.progress.keys()].filter((f) => !ui.catalog.some((m) => filenameFromUrl(m.url) === f));
  $("downloaded").replaceChildren(
    ...custom.map((filename) => el("li", {}, el("span", { className: "name", textContent: filename }), ...progressFor(filename))),
    ...(ui.downloaded.length === 0 ? [el("li", { textContent: "Nenhum modelo baixado ainda." })] : []),
    ...ui.downloaded.map((model) => el("li", {},
      el("label", { className: "name" },
        el("input", { type: "radio", name: "model", checked: model.filename === ui.selected, onchange: () => { ui.selected = model.filename; render(); } }),
        ` ${model.filename} — ${formatBytes(model.size)}`),
      el("button", { type: "button", textContent: "Apagar", onclick: () => deleteModel(model.filename) }))));

  const { status, model, error } = ui.llama;
  $("status").textContent = `${STATUS_TEXT[status]}${model ? ` (${model})` : ""}${error ? `: ${error}` : ""}`;
  $("start").disabled = !ui.selected || status === "starting";
  $("start").textContent = status === "ready" || status === "starting" ? "Reiniciar com o selecionado" : "Iniciar";
  $("stop").disabled = status === "stopped";
  $("logs").textContent = ui.logs.join("\n");
  $("logs").scrollTop = $("logs").scrollHeight;
}

async function deleteModel(filename) {
  showError();
  if (!confirm(`Apagar ${filename}?`)) return;
  await invoke("delete_model", { filename }).catch(showError);
  await refreshDownloaded();
}

$("custom").addEventListener("submit", (event) => {
  event.preventDefault();
  const url = $("custom-url").value.trim();
  $("custom-url").value = "";
  download(url);
});

$("start").addEventListener("click", () => {
  showError();
  invoke("start_llama", { filename: ui.selected }).catch(showError);
});

$("stop").addEventListener("click", () => invoke("stop_llama").catch(showError));

$("workshop").addEventListener("click", async () => {
  showError();
  $("workshop").disabled = true;
  $("workshop-status").textContent = "Iniciando o Workshop (a primeira vez pode levar alguns minutos)…";
  try {
    await invoke("open_workshop");
    $("workshop-status").textContent = "";
  } catch (error) {
    $("workshop-status").textContent = "";
    showError(error);
  } finally {
    $("workshop").disabled = false;
  }
});

await listen("llama-state", ({ payload }) => {
  ui.llama = payload;
  render();
});
await listen("llama-log", ({ payload }) => {
  ui.logs = [...ui.logs.slice(-(MAX_LOGS - 1)), payload];
  render();
});
await listen("download-progress", ({ payload }) => {
  if (ui.progress.has(payload.filename)) ui.progress.set(payload.filename, payload);
  render();
});

ui.catalog = await invoke("list_catalog");
const status = await invoke("llama_status");
ui.llama = status.state;
ui.logs = status.logs;
await refreshDownloaded();
