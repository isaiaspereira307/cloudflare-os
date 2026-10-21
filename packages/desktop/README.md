# Gadgets Desktop

App desktop (Tauri v2) que embute o `llama-server` do llama.cpp, baixa um modelo GGUF e abre o Workshop.

Fase 1: o Workshop ainda não vem empacotado. O app reaproveita um Workshop que já responda em
`http://localhost:8787`; se não houver, roda `pnpm run-local` na raiz do repositório
(`GADGETS_REPO_ROOT`, ou a raiz deste checkout) e encerra esse processo ao sair.

## Pré-requisitos

- Rust (stable) e pnpm.
- Linux: `sudo apt install libwebkit2gtk-4.1-dev libsoup-3.0-dev build-essential libssl-dev libayatana-appindicator3-dev librsvg2-dev`
  Se outro `pkg-config` (por exemplo o do Homebrew) vier antes no `PATH` e não achar essas libs,
  rode com `PKG_CONFIG=/usr/bin/pkg-config`.

## Uso

```sh
pnpm install
pnpm --filter @gadgets/desktop fetch-llama   # baixa o llama.cpp fixado (b11457) em src-tauri/resources/llama/
pnpm --filter @gadgets/desktop dev
```

No app, baixe um modelo do catálogo (ou cole a URL https de um GGUF), selecione-o e clique em
Iniciar. Quando o status ficar "pronto", clique em Abrir Workshop e, lá, adicione um modelo com o
provedor **llama.cpp** e a URL `http://127.0.0.1:8080`.

Os modelos ficam em `<app data>/models/` (no Linux, `~/.local/share/dev.gadgets.desktop/models/`).
Um download interrompido continua de onde parou ao clicar em Baixar de novo.

## Testes

```sh
cd packages/desktop/src-tauri && cargo test -p desktop-core
```

`desktop-core` guarda a lógica pura (validação de nome/URL, catálogo e a máquina de estados do
llama-server) e compila sem webkit.
