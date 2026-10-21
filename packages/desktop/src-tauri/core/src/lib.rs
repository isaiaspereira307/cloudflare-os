use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::Path;

/// One of the three model tiers the app offers. Only one is installed at a time.
#[derive(Serialize)]
pub struct CatalogModel {
    pub id: &'static str,
    pub name: &'static str,
    pub model: &'static str,
    pub note: &'static str,
    pub filename: &'static str,
    pub url: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

/// The tier preselected when no model is installed: the smallest.
pub const DEFAULT_MODEL: &str = "low";

// Sizes and sha256 digests are the LFS metadata Hugging Face reports for each file.
pub const CATALOG: &[CatalogModel] = &[
    CatalogModel {
        id: "low",
        name: "Low",
        model: "Qwen3.5 0.8B (Q4_0)",
        note: "Leve e rápido: roda em quase qualquer máquina (cerca de 2 GB de RAM livre). Bom para testar e para pedidos simples; erra mais em código e em chamadas de ferramenta.",
        filename: "Qwen3.5-0.8B-Q4_0.gguf",
        url: "https://huggingface.co/unsloth/Qwen3.5-0.8B-GGUF/resolve/main/Qwen3.5-0.8B-Q4_0.gguf",
        size: 507_154_688,
        sha256: "444406ddd926550c724ec18d5120a9d40ded44908a063b0e66e9a7e5464c652c",
    },
    CatalogModel {
        id: "medium",
        name: "Medium",
        model: "Qwen3.5 4B (Q4_0)",
        note: "Equilíbrio entre qualidade e velocidade. Recomendado com 8 GB de RAM ou mais.",
        filename: "Qwen3.5-4B-Q4_0.gguf",
        url: "https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/resolve/main/Qwen3.5-4B-Q4_0.gguf",
        size: 2_583_221_408,
        sha256: "298fcb5fe7a77ccc79745ae24751560c5ac56874caff4bb39b1f2055bd72b8bb",
    },
    CatalogModel {
        id: "high",
        name: "High",
        model: "Qwen3.5 9B (Q4_0)",
        note: "Melhor qualidade para criar gadgets, porém mais lento. Recomendado com 16 GB de RAM ou uma GPU.",
        filename: "Qwen3.5-9B-Q4_0.gguf",
        url: "https://huggingface.co/unsloth/Qwen3.5-9B-GGUF/resolve/main/Qwen3.5-9B-Q4_0.gguf",
        size: 5_379_417_312,
        sha256: "17670346b4260ddcb0173965145155885024f3c9a4a24389a3370751edbcde24",
    },
];

pub fn catalog_model(id: &str) -> Result<&'static CatalogModel, String> {
    CATALOG.iter().find(|m| m.id == id).ok_or_else(|| format!("Modelo desconhecido: {id}"))
}

#[derive(Serialize, Clone, Debug, PartialEq, Default)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum LlamaState {
    #[default]
    Stopped,
    Starting { model: String },
    Ready { model: String },
    Failed { error: String },
}

#[derive(Debug)]
pub enum LlamaEvent {
    Start(String),
    Healthy,
    Exited(String),
    Stop,
}

impl LlamaState {
    pub fn next(&self, event: LlamaEvent) -> Result<LlamaState, String> {
        use LlamaState::*;
        match (self, event) {
            (Stopped | Failed { .. }, LlamaEvent::Start(model)) => Ok(Starting { model }),
            (Starting { model }, LlamaEvent::Healthy) => Ok(Ready { model: model.clone() }),
            (Starting { .. } | Ready { .. }, LlamaEvent::Exited(error)) => Ok(Failed { error }),
            (_, LlamaEvent::Stop) => Ok(Stopped),
            (state, event) => Err(format!("Transição inválida: {event:?} em {state:?}")),
        }
    }

    pub fn model(&self) -> Option<&str> {
        match self {
            LlamaState::Starting { model } | LlamaState::Ready { model } => Some(model),
            _ => None,
        }
    }
}

/// `runtime.json`, written by `scripts/build-runtime.ts` next to the workerd config.
#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeManifest {
    pub port: u16,
    /// The disk service that serves the frontend; the config names it by a build-machine path.
    pub assets_service: String,
    /// Directories, relative to workerd's working directory, that its disk services need to exist.
    pub dirs: Vec<String>,
}

impl RuntimeManifest {
    pub fn origin(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Arguments for `workerd` started in the state directory, so the config's relative
    /// persistence paths land there.
    pub fn workerd_args(&self, runtime_dir: &Path) -> Vec<OsString> {
        let mut assets = OsString::from(format!("--directory-path={}=", self.assets_service));
        assets.push(runtime_dir.join("assets"));
        vec![
            "serve".into(),
            "--binary".into(),
            // Miniflare's simulators use compatibility flags that have no default-on date.
            "--experimental".into(),
            runtime_dir.join("config.bin").into(),
            assets,
            format!("--socket-addr=entry=127.0.0.1:{}", self.port).into(),
            // Miniflare's Node-side loopback server, which only dev-time features call. Port 9
            // (discard) refuses connections, so those calls fail fast instead of reaching anything.
            "--external-addr=loopback=127.0.0.1:9".into(),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog() {
        assert_eq!(CATALOG.iter().map(|m| m.name).collect::<Vec<_>>(), ["Low", "Medium", "High"]);
        let default = catalog_model(DEFAULT_MODEL).unwrap();
        assert!(CATALOG.iter().all(|m| m.size >= default.size), "the default is the smallest");
        for m in CATALOG {
            assert!(m.url.starts_with("https://") && m.url.ends_with(&format!("/{}", m.filename)), "{}", m.id);
            assert!(m.filename.contains("Q4_0") && m.sha256.len() == 64, "{}", m.id);
        }
        assert!(catalog_model("x").is_err());
    }

    #[test]
    fn transitions() {
        let m = || "m.gguf".to_string();
        let starting = LlamaState::Stopped.next(LlamaEvent::Start(m())).unwrap();
        assert_eq!(starting, LlamaState::Starting { model: m() });
        let ready = starting.next(LlamaEvent::Healthy).unwrap();
        assert_eq!(ready, LlamaState::Ready { model: m() });
        assert_eq!(ready.model(), Some("m.gguf"));
        assert!(ready.next(LlamaEvent::Start(m())).is_err(), "no double start");
        assert!(ready.next(LlamaEvent::Healthy).is_err());
        let failed = ready.next(LlamaEvent::Exited("boom".into())).unwrap();
        assert_eq!(failed, LlamaState::Failed { error: "boom".into() });
        assert!(failed.next(LlamaEvent::Healthy).is_err());
        assert!(LlamaState::Stopped.next(LlamaEvent::Exited("x".into())).is_err());
        let restarted = failed.next(LlamaEvent::Start(m())).unwrap();
        assert_eq!(restarted, LlamaState::Starting { model: m() });
        assert_eq!(restarted.next(LlamaEvent::Stop).unwrap(), LlamaState::Stopped);
    }

    #[test]
    fn workerd_args() {
        let manifest: RuntimeManifest =
            serde_json::from_str(r#"{"port": 8797, "assetsService": "assets:storage", "dirs": ["state/do"]}"#).unwrap();
        assert_eq!(manifest.origin(), "http://127.0.0.1:8797");
        let args: Vec<String> = manifest
            .workerd_args(Path::new("/opt/app/workshop"))
            .into_iter()
            .map(|arg| arg.into_string().unwrap())
            .collect();
        assert_eq!(args[3], "/opt/app/workshop/config.bin");
        assert!(args.contains(&"--directory-path=assets:storage=/opt/app/workshop/assets".to_string()));
        assert!(args.contains(&"--socket-addr=entry=127.0.0.1:8797".to_string()));
    }
}
