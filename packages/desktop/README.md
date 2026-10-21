# Gadgets Desktop

App desktop (Tauri v2) que embute o `llama-server` do llama.cpp, baixa um modelo GGUF e roda o
Workshop localmente, sem precisar do repositório, do pnpm nem de um Node instalado.

O Workshop vem empacotado em `resources/workshop/`: o binário `workerd` (o runtime dos Workers) e uma
configuração gerada que contém todos os workers do app. O app inicia o `workerd` ao abrir, em
`http://127.0.0.1:8797` (outra porta que a do `pnpm run-local`, 8787, para os dois poderem rodar
juntos), e o encerra ao sair. Os dados (contas, workspaces, KV, R2, Durable Objects) ficam em
`<app data>/workshop/` e sobrevivem a reinícios; o log do runtime fica em
`<app data>/workshop/workerd.log`.

Workers incluídos: `router`, `workshop-backend` e os gatekeepers que não precisam de credenciais
OAuth de terceiros (`gatekeeper-context`, `gatekeeper-scheduler`, `gatekeeper-mcp`,
`gatekeeper-homeassistant`). Os demais (GitHub, Google, Slack etc.) ficam de fora. Recursos que
dependem de bindings exclusivos da Cloudflare não existem no desktop: Workers AI (o agente usa um
provedor configurado, como o llama.cpp) e o Browser Run (a exportação de Gadgets em PDF responde
"Gadget export is not configured for this deployment"). O porquê de cada escolha está em
[docs/phase2-decisions.md](docs/phase2-decisions.md).

## Pré-requisitos (para compilar)

- Rust (stable), pnpm e as dependências do repositório (`pnpm install`).
- Linux: `sudo apt install libwebkit2gtk-4.1-dev libsoup-3.0-dev build-essential libssl-dev libayatana-appindicator3-dev librsvg2-dev`
  Se outro `pkg-config` (por exemplo o do Homebrew) vier antes no `PATH` e não achar essas libs,
  rode com `PKG_CONFIG=/usr/bin/pkg-config`.

## Uso

```sh
pnpm install
pnpm --filter @gadgets/desktop fetch-llama     # llama.cpp fixado (b11457) em src-tauri/resources/llama/
pnpm --filter @gadgets/desktop build-runtime   # Workshop em src-tauri/resources/workshop/
pnpm --filter @gadgets/desktop dev             # ou: pnpm --filter @gadgets/desktop tauri build --bundles deb
```

Rode `build-runtime` de novo sempre que mudar o código do Workshop. Ele empacota cada worker como o
release faz (`wrangler deploy --dry-run`) e falha se a configuração gerada apontar para algum caminho
desta máquina.

No app, escolha um nível de modelo e clique em Baixar; depois clique em Iniciar. Os níveis são
Qwen3.5 em Q4_0, da [unsloth](https://huggingface.co/unsloth):

| Nível | Modelo | Tamanho |
| --- | --- | --- |
| Low (padrão) | Qwen3.5 0.8B | 507 MB |
| Medium | Qwen3.5 4B | 2,6 GB |
| High | Qwen3.5 9B | 5,4 GB |

Só um modelo fica instalado: baixar outro nível apaga o anterior (e reinicia o llama-server se ele
estava rodando). O download é conferido pelo sha256 publicado pelo Hugging Face. Quando o status ficar "pronto", clique em Abrir Workshop, crie sua conta e, lá, adicione um
modelo com o provedor **llama.cpp** e a URL `http://127.0.0.1:8089`. Para acessar o painel `/admin`,
crie a conta com o usuário `admin`, como no `run-local`.

Os modelos ficam em `<app data>/models/` (no Linux, `~/.local/share/dev.gadgets.desktop/models/`).
Um download interrompido continua de onde parou ao clicar em Baixar de novo.

## Testes

```sh
cd packages/desktop/src-tauri && cargo test -p desktop-core
```

`desktop-core` guarda a lógica pura (o catálogo dos três níveis, a máquina de estados do
llama-server e os argumentos do `workerd`) e compila sem webkit.
