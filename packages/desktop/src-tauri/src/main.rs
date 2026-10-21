#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use desktop_core::{validate_filename, validate_url, CatalogModel, LlamaEvent, LlamaState, RuntimeManifest, CATALOG};
use serde::Serialize;
use std::collections::{HashSet, VecDeque};
use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};
use tokio::io::AsyncWriteExt;

/// The Workshop's llama.cpp provider defaults to http://127.0.0.1:8089.
const LLAMA_PORT: &str = "8089";
const LOG_LINES: usize = 40;
#[cfg(windows)]
const LLAMA_BIN: &str = "llama-server.exe";
#[cfg(not(windows))]
const LLAMA_BIN: &str = "llama-server";
#[cfg(windows)]
const WORKERD_BIN: &str = "workerd.exe";
#[cfg(not(windows))]
const WORKERD_BIN: &str = "workerd";

#[derive(Default)]
struct Llama {
    state: LlamaState,
    child: Option<Child>,
    /// Bumped on every start/stop so a monitor task of an older process stands down.
    generation: u64,
    logs: VecDeque<String>,
}

#[derive(Default)]
struct AppState {
    llama: Mutex<Llama>,
    downloads: Mutex<HashSet<String>>,
    workshop: Mutex<Option<Child>>,
}

#[derive(Serialize)]
struct LlamaStatus {
    state: LlamaState,
    logs: Vec<String>,
}

#[derive(Serialize, Clone)]
struct DownloadProgress<'a> {
    filename: &'a str,
    received: u64,
    total: Option<u64>,
}

#[derive(Serialize)]
struct Downloaded {
    filename: String,
    size: u64,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn models_dir(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(err)?.join("models"))
}

fn transition(app: &AppHandle, llama: &mut Llama, event: LlamaEvent) -> Result<(), String> {
    llama.state = llama.state.next(event)?;
    let _ = app.emit("llama-state", &llama.state);
    Ok(())
}

fn stop(app: &AppHandle, llama: &mut Llama) {
    if let Some(mut child) = llama.child.take() {
        let _ = child.kill();
        let _ = child.wait();
    }
    llama.generation += 1;
    let _ = transition(app, llama, LlamaEvent::Stop);
}

#[tauri::command]
fn list_catalog() -> &'static [CatalogModel] {
    CATALOG
}

#[tauri::command]
fn list_downloaded(app: AppHandle) -> Result<Vec<Downloaded>, String> {
    let Ok(entries) = std::fs::read_dir(models_dir(&app)?) else {
        return Ok(vec![]);
    };
    let mut models: Vec<Downloaded> = entries
        .flatten()
        .filter_map(|entry| {
            let filename = entry.file_name().into_string().ok()?;
            validate_filename(&filename).ok()?;
            Some(Downloaded { size: entry.metadata().ok()?.len(), filename })
        })
        .collect();
    models.sort_by(|a, b| a.filename.cmp(&b.filename));
    Ok(models)
}

#[tauri::command]
async fn download_model(app: AppHandle, url: String, filename: String) -> Result<(), String> {
    validate_url(&url)?;
    validate_filename(&filename)?;
    let state = app.state::<AppState>();
    if !state.downloads.lock().unwrap().insert(filename.clone()) {
        return Err(format!("{filename} já está sendo baixado"));
    }
    let result = download(&app, &url, &filename).await;
    state.downloads.lock().unwrap().remove(&filename);
    result
}

/// Streams into `<file>.part`, resuming with a Range request when a partial file exists.
async fn download(app: &AppHandle, url: &str, filename: &str) -> Result<(), String> {
    let dir = models_dir(app)?;
    tokio::fs::create_dir_all(&dir).await.map_err(err)?;
    let part = dir.join(format!("{filename}.part"));
    let offset = tokio::fs::metadata(&part).await.map(|m| m.len()).unwrap_or(0);

    let client = reqwest::Client::builder().https_only(true).build().map_err(err)?;
    let mut request = client.get(url);
    if offset > 0 {
        request = request.header(reqwest::header::RANGE, format!("bytes={offset}-"));
    }
    let mut response = request.send().await.map_err(err)?;
    let status = response.status();
    let (mut file, mut received) = if status == reqwest::StatusCode::PARTIAL_CONTENT {
        let file = tokio::fs::OpenOptions::new().append(true).open(&part).await.map_err(err)?;
        (file, offset)
    } else if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE && offset > 0 {
        // The partial file already holds every byte.
        return tokio::fs::rename(&part, dir.join(filename)).await.map_err(err);
    } else if status.is_success() {
        (tokio::fs::File::create(&part).await.map_err(err)?, 0)
    } else {
        return Err(format!("Falha no download: HTTP {status}"));
    };

    let total = response.content_length().map(|len| len + received);
    let mut reported = received;
    while let Some(chunk) = response.chunk().await.map_err(err)? {
        file.write_all(&chunk).await.map_err(err)?;
        received += chunk.len() as u64;
        if received - reported >= 4 << 20 {
            reported = received;
            let _ = app.emit("download-progress", DownloadProgress { filename, received, total });
        }
    }
    file.flush().await.map_err(err)?;
    drop(file);
    if total.is_some_and(|total| total != received) {
        return Err(format!("Download incompleto ({received} de {total:?} bytes); tente novamente para continuar"));
    }
    tokio::fs::rename(&part, dir.join(filename)).await.map_err(err)?;
    let _ = app.emit("download-progress", DownloadProgress { filename, received, total });
    Ok(())
}

#[tauri::command]
fn delete_model(app: AppHandle, filename: String) -> Result<(), String> {
    validate_filename(&filename)?;
    if app.state::<AppState>().llama.lock().unwrap().state.model() == Some(filename.as_str()) {
        return Err("Pare o llama-server antes de apagar o modelo em uso".into());
    }
    std::fs::remove_file(models_dir(&app)?.join(filename)).map_err(err)
}

#[tauri::command]
fn start_llama(app: AppHandle, filename: String) -> Result<(), String> {
    validate_filename(&filename)?;
    let model = models_dir(&app)?.join(&filename);
    if !model.is_file() {
        return Err(format!("Modelo não encontrado: {filename}"));
    }
    let dir = app.path().resource_dir().map_err(err)?.join("llama");

    let state = app.state::<AppState>();
    let mut llama = state.llama.lock().unwrap();
    stop(&app, &mut llama);
    llama.logs.clear();
    transition(&app, &mut llama, LlamaEvent::Start(filename))?;

    let mut command = Command::new(dir.join(LLAMA_BIN));
    command
        .arg("-m")
        .arg(&model)
        .args(["--host", "127.0.0.1", "--port", LLAMA_PORT, "--jinja", "-c", "32768", "-ngl", "99"])
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // The release ships its shared libraries next to the binary; Windows finds DLLs there already.
    #[cfg(target_os = "linux")]
    command.env("LD_LIBRARY_PATH", &dir);
    #[cfg(target_os = "macos")]
    command.env("DYLD_LIBRARY_PATH", &dir);

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(e) => {
            let error = format!("Não foi possível iniciar {}: {e}", dir.join(LLAMA_BIN).display());
            transition(&app, &mut llama, LlamaEvent::Exited(error.clone()))?;
            return Err(error);
        }
    };
    pipe_logs(&app, child.stdout.take());
    pipe_logs(&app, child.stderr.take());
    llama.child = Some(child);
    tauri::async_runtime::spawn(monitor(app.clone(), llama.generation));
    Ok(())
}

fn pipe_logs(app: &AppHandle, stream: Option<impl Read + Send + 'static>) {
    let Some(stream) = stream else { return };
    let app = app.clone();
    std::thread::spawn(move || {
        for line in BufReader::new(stream).lines().map_while(Result::ok) {
            let _ = app.emit("llama-log", &line);
            let state = app.state::<AppState>();
            let mut llama = state.llama.lock().unwrap();
            if llama.logs.len() == LOG_LINES {
                llama.logs.pop_front();
            }
            llama.logs.push_back(line);
        }
    });
}

/// Moves Starting to Ready once /health answers 200, and any running state to Failed if the
/// process exits on its own.
async fn monitor(app: AppHandle, generation: u64) {
    let client = reqwest::Client::builder().timeout(Duration::from_secs(2)).build().unwrap();
    let health = format!("http://127.0.0.1:{LLAMA_PORT}/health");
    loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let starting = {
            let state = app.state::<AppState>();
            let mut llama = state.llama.lock().unwrap();
            if llama.generation != generation {
                return;
            }
            let Some(child) = llama.child.as_mut() else { return };
            if let Ok(Some(status)) = child.try_wait() {
                let last = llama.logs.back().cloned().unwrap_or_default();
                llama.child = None;
                let _ = transition(&app, &mut llama, LlamaEvent::Exited(format!("llama-server saiu ({status}): {last}")));
                return;
            }
            matches!(llama.state, LlamaState::Starting { .. })
        };
        if starting && client.get(&health).send().await.is_ok_and(|r| r.status().is_success()) {
            let state = app.state::<AppState>();
            let mut llama = state.llama.lock().unwrap();
            if llama.generation == generation {
                let _ = transition(&app, &mut llama, LlamaEvent::Healthy);
            }
        }
    }
}

#[tauri::command]
fn stop_llama(app: AppHandle) {
    stop(&app, &mut app.state::<AppState>().llama.lock().unwrap());
}

#[tauri::command]
fn llama_status(app: AppHandle) -> LlamaStatus {
    let state = app.state::<AppState>();
    let llama = state.llama.lock().unwrap();
    LlamaStatus { state: llama.state.clone(), logs: llama.logs.iter().cloned().collect() }
}

fn workshop_log(app: &AppHandle) -> Result<PathBuf, String> {
    Ok(app.path().app_data_dir().map_err(err)?.join("workshop").join("workerd.log"))
}

/// Starts the bundled Workshop (`resources/workshop`, built by scripts/build-runtime.ts) unless it
/// is already running. Its state lives in `<app data>/workshop`, workerd's working directory.
fn start_workshop(app: &AppHandle) -> Result<String, String> {
    let runtime_dir = app.path().resource_dir().map_err(err)?.join("workshop");
    let manifest: RuntimeManifest = serde_json::from_slice(
        &std::fs::read(runtime_dir.join("runtime.json"))
            .map_err(|e| format!("Workshop não empacotado ({}): {e}", runtime_dir.display()))?,
    )
    .map_err(err)?;
    let state = app.state::<AppState>();
    let mut workshop = state.workshop.lock().unwrap();
    if workshop.as_mut().is_some_and(|child| child.try_wait().is_ok_and(|status| status.is_none())) {
        return Ok(manifest.origin());
    }

    let data_dir = app.path().app_data_dir().map_err(err)?.join("workshop");
    std::fs::create_dir_all(&data_dir).map_err(err)?;
    for dir in &manifest.dirs {
        std::fs::create_dir_all(data_dir.join(dir)).map_err(err)?;
    }
    let log = std::fs::File::create(workshop_log(app)?).map_err(err)?;
    let child = Command::new(runtime_dir.join(WORKERD_BIN))
        .args(manifest.workerd_args(&runtime_dir))
        .current_dir(&data_dir)
        .stdin(Stdio::null())
        .stdout(log.try_clone().map_err(err)?)
        .stderr(log)
        .spawn()
        .map_err(|e| format!("Não foi possível iniciar o Workshop: {e}"))?;
    *workshop = Some(child);
    Ok(manifest.origin())
}

/// The end of workerd's log, which names the problem when it fails to start (e.g. the port is taken).
fn log_tail(app: &AppHandle) -> String {
    let log = workshop_log(app).and_then(|path| std::fs::read_to_string(path).map_err(err)).unwrap_or_default();
    let start = log.len().saturating_sub(800);
    log.get(start..).unwrap_or(&log).trim().to_string()
}

#[tauri::command]
async fn open_workshop(app: AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_webview_window("workshop") {
        return window.set_focus().map_err(err);
    }
    let origin = start_workshop(&app)?;
    let client = reqwest::Client::builder().timeout(Duration::from_secs(2)).build().map_err(err)?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while !client.get(&origin).send().await.is_ok_and(|r| r.status().is_success()) {
        let exited = app.state::<AppState>().workshop.lock().unwrap().as_mut().and_then(|child| child.try_wait().ok().flatten());
        if let Some(status) = exited {
            return Err(format!("O Workshop saiu ({status}): {}", log_tail(&app)));
        }
        if tokio::time::Instant::now() > deadline {
            return Err(format!("O Workshop não respondeu em {origin}: {}", log_tail(&app)));
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    WebviewWindowBuilder::new(&app, "workshop", WebviewUrl::External(origin.parse().map_err(err)?))
        .title("Gadgets Workshop")
        .inner_size(1280.0, 860.0)
        .build()
        .map_err(err)?;
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .manage(AppState::default())
        .invoke_handler(tauri::generate_handler![
            list_catalog,
            list_downloaded,
            download_model,
            delete_model,
            start_llama,
            stop_llama,
            llama_status,
            open_workshop
        ])
        // Ctrl-C or SIGTERM would otherwise skip RunEvent::Exit and orphan workerd and llama-server.
        .setup(|app| {
            // Started now so it is usually up by the time the user clicks Abrir Workshop, which
            // reports any failure.
            if let Err(e) = start_workshop(app.handle()) {
                eprintln!("{e}");
            }
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let _ = tokio::signal::ctrl_c().await;
                handle.exit(0);
            });
            #[cfg(unix)]
            {
                use tokio::signal::unix::{signal, SignalKind};
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Ok(mut terminate) = signal(SignalKind::terminate()) {
                        terminate.recv().await;
                        handle.exit(0);
                    }
                });
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build the Tauri app")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                let state = app.state::<AppState>();
                let llama = state.llama.lock().unwrap().child.take();
                let workshop = state.workshop.lock().unwrap().take();
                for mut child in llama.into_iter().chain(workshop) {
                    let _ = child.kill();
                    let _ = child.wait();
                }
            }
        });
}
