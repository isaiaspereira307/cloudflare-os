const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const STATUS_TEXT = { stopped: "parado", starting: "iniciando…", ready: "pronto", failed: "erro" };
const MAX_LOGS = 40;

const ui = {
  models: [],
  installed: null,
  downloading: null,
  selected: null,
  progress: null,
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
  return bytes >= 1e9 ? `${(bytes / 1e9).toFixed(1).replace(".", ",")} GB` : `${(bytes / 1e6).toFixed(0)} MB`;
}

function showError(error) {
  $("error").textContent = error ? String(error) : "";
}

async function refreshCatalog() {
  const catalog = await invoke("catalog");
  ui.models = catalog.models;
  ui.installed = catalog.installed;
  ui.downloading = catalog.downloading;
  ui.selected ??= catalog.installed ?? catalog.default;
  render();
}

async function download(id) {
  showError();
  ui.downloading = id;
  ui.progress = null;
  render();
  try {
    await invoke("download_model", { id });
  } catch (error) {
    showError(error);
  } finally {
    ui.progress = null;
    await refreshCatalog();
  }
}

function render() {
  $("catalog").replaceChildren(...ui.models.map((model) => {
    const badge = model.id === ui.installed ? " (instalado)" : "";
    const progress = model.id === ui.downloading
      ? [el("progress", { max: model.size, value: ui.progress?.received ?? 0 }),
         el("span", { textContent: `${formatBytes(ui.progress?.received ?? 0)} de ${formatBytes(model.size)}` })]
      : [];
    return el("li", {},
      el("label", {},
        el("input", { type: "radio", name: "model", value: model.id, checked: model.id === ui.selected, onchange: () => { ui.selected = model.id; render(); } }),
        el("strong", { textContent: ` ${model.name}` }),
        ` · ${model.model} · ${formatBytes(model.size)}${badge}`,
        el("span", { className: "note", textContent: model.note })),
      ...progress);
  }));

  const selected = ui.models.find((m) => m.id === ui.selected);
  const button = $("download");
  button.hidden = !selected || selected.id === ui.installed;
  button.disabled = ui.downloading !== null;
  button.textContent = ui.downloading
    ? "Baixando…"
    : ui.installed ? `Trocar para ${selected?.name}` : `Baixar ${selected?.name}`;

  const { status, model, error } = ui.llama;
  $("status").textContent = `${STATUS_TEXT[status]}${model ? ` (${model})` : ""}${error ? `: ${error}` : ""}`;
  $("start").disabled = !ui.installed || status === "starting";
  $("start").textContent = status === "ready" || status === "starting" ? "Reiniciar" : "Iniciar";
  $("stop").disabled = status === "stopped";
  $("logs").textContent = ui.logs.join("\n");
  $("logs").scrollTop = $("logs").scrollHeight;
}

$("download").addEventListener("click", () => download(ui.selected));

$("start").addEventListener("click", () => {
  showError();
  invoke("start_llama").catch(showError);
});

$("stop").addEventListener("click", () => invoke("stop_llama").catch(showError));

$("workshop").addEventListener("click", async () => {
  showError();
  $("workshop").disabled = true;
  $("workshop-status").textContent = "Iniciando o Workshop…";
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
  if (payload.status === "starting") ui.logs = [];
  ui.llama = payload;
  render();
});
await listen("llama-log", ({ payload }) => {
  ui.logs = [...ui.logs.slice(-(MAX_LOGS - 1)), payload];
  render();
});
await listen("download-progress", ({ payload }) => {
  if (payload.id === ui.downloading) ui.progress = payload;
  render();
});

const status = await invoke("llama_status");
ui.llama = status.state;
ui.logs = status.logs;
await refreshCatalog();
