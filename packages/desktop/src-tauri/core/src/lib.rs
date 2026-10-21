use serde::Serialize;

#[derive(Serialize)]
pub struct CatalogModel {
    pub id: &'static str,
    pub name: &'static str,
    pub url: &'static str,
    pub size: u64,
}

// Sizes are the Content-Length Hugging Face reported for each URL.
pub const CATALOG: &[CatalogModel] = &[
    CatalogModel {
        id: "qwen3-4b",
        name: "Qwen3 4B (Q4_K_M)",
        url: "https://huggingface.co/Qwen/Qwen3-4B-GGUF/resolve/main/Qwen3-4B-Q4_K_M.gguf",
        size: 2_497_280_256,
    },
    CatalogModel {
        id: "gemma3-4b",
        name: "Gemma 3 4B it (Q4_K_M)",
        url: "https://huggingface.co/ggml-org/gemma-3-4b-it-GGUF/resolve/main/gemma-3-4b-it-Q4_K_M.gguf",
        size: 2_489_757_856,
    },
    CatalogModel {
        id: "qwen2.5-coder-7b",
        name: "Qwen2.5-Coder 7B Instruct (Q4_K_M)",
        url: "https://huggingface.co/bartowski/Qwen2.5-Coder-7B-Instruct-GGUF/resolve/main/Qwen2.5-Coder-7B-Instruct-Q4_K_M.gguf",
        size: 4_683_074_336,
    },
    CatalogModel {
        id: "qwen3-8b",
        name: "Qwen3 8B (Q4_K_M)",
        url: "https://huggingface.co/Qwen/Qwen3-8B-GGUF/resolve/main/Qwen3-8B-Q4_K_M.gguf",
        size: 5_027_783_488,
    },
];

/// A model file name is joined onto the models directory, so it must stay a single path segment.
pub fn validate_filename(name: &str) -> Result<(), String> {
    let stem = name.strip_suffix(".gguf").unwrap_or("");
    if stem.is_empty()
        || stem.starts_with('.')
        || name.chars().any(|c| matches!(c, '/' | '\\' | ':' | '\0'))
    {
        return Err(format!("Nome de arquivo inválido: {name}"));
    }
    Ok(())
}

pub fn validate_url(url: &str) -> Result<(), String> {
    match url.strip_prefix("https://") {
        Some(rest) if !rest.is_empty() && !url.contains(char::is_whitespace) => Ok(()),
        _ => Err("Apenas URLs https:// são aceitas".into()),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filenames() {
        assert!(validate_filename("Qwen3-4B-Q4_K_M.gguf").is_ok());
        for bad in ["../x.gguf", "a/b.gguf", "a\\b.gguf", "C:x.gguf", "x.bin", ".gguf", ".hidden.gguf", "x.gguf.part", ""] {
            assert!(validate_filename(bad).is_err(), "{bad} should be rejected");
        }
        for model in CATALOG {
            let file = model.url.rsplit('/').next().unwrap();
            assert!(validate_filename(file).is_ok() && validate_url(model.url).is_ok(), "{}", model.id);
        }
    }

    #[test]
    fn urls() {
        assert!(validate_url("https://huggingface.co/x.gguf").is_ok());
        for bad in ["http://huggingface.co/x.gguf", "file:///etc/passwd", "https://", "https://a b", "HTTPS://x"] {
            assert!(validate_url(bad).is_err(), "{bad} should be rejected");
        }
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
}
