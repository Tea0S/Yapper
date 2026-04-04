use crate::db::get_setting;
use crate::open_db;
use crate::paths::yapper_node_main_path;
use crate::sidecar::python_executable;
use crate::state::AppState;
use serde::Serialize;
use std::collections::VecDeque;
use std::process::Stdio;
use std::sync::Arc;
use tauri::State;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::Mutex;

const LOG_CAP: usize = 48;

async fn push_log(logs: &Arc<Mutex<VecDeque<String>>>, line: String) {
    let mut q = logs.lock().await;
    while q.len() >= LOG_CAP {
        q.pop_front();
    }
    q.push_back(line);
}

fn bind_host_from_setting(s: Option<String>) -> &'static str {
    match s.as_deref() {
        Some("loopback") => "127.0.0.1",
        _ => "0.0.0.0",
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeServerStatus {
    pub running: bool,
    pub bind_mode: String,
    pub bind_host: String,
    pub port: u16,
    pub token_configured: bool,
    /// URLs to enter in another Yapper install (Settings → remote).
    pub suggested_client_urls: Vec<String>,
    pub log_tail: Vec<String>,
    pub script_found: bool,
    pub script_path: String,
}

fn suggested_urls(bind_host: &str, port: u16) -> Vec<String> {
    let mut out = Vec::new();
    if bind_host == "127.0.0.1" {
        out.push(format!("ws://127.0.0.1:{port}"));
        return out;
    }
    out.push(format!("ws://127.0.0.1:{port}"));
    if let Ok(ip) = local_ip_address::local_ip() {
        out.push(format!("ws://{ip}:{port}"));
    }
    out
}

async fn read_settings(app: &tauri::AppHandle) -> Result<(String, u16, String, String), String> {
    let conn = open_db(app)?;
    let bind_mode = get_setting(&conn, "node_server_bind")
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| "lan".into());
    let host = bind_host_from_setting(Some(bind_mode.clone())).to_string();
    let port_s = get_setting(&conn, "node_server_port")
        .map_err(|e| e.to_string())?
        .unwrap_or_else(|| "8765".into());
    let port: u16 = port_s
        .trim()
        .parse()
        .map_err(|_| "Invalid port (use 1024–65535)".to_string())?;
    let token = get_setting(&conn, "node_server_token")
        .map_err(|e| e.to_string())?
        .unwrap_or_default();
    Ok((bind_mode, port, host, token))
}

/// Sync process state: drop handle if the child has exited.
async fn reconcile_child(state: &AppState) {
    let mut guard = state.yapper_node.lock().await;
    if let Some(mut c) = guard.take() {
        match c.try_wait() {
            Ok(Some(_)) => {}
            Ok(None) => {
                *guard = Some(c);
            }
            Err(_) => {
                *guard = Some(c);
            }
        }
    }
}

#[tauri::command]
pub async fn yapper_node_status(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<NodeServerStatus, String> {
    reconcile_child(&state).await;
    let script = yapper_node_main_path(&app);
    let script_found = script.is_file();
    let script_path = script.to_string_lossy().into_owned();

    let (bind_mode, port, bind_host, token) = read_settings(&app).await?;
    let running = state.yapper_node.lock().await.is_some();
    let logs = state.yapper_node_logs.lock().await;
    let log_tail: Vec<String> = logs.iter().cloned().collect();

    Ok(NodeServerStatus {
        running,
        bind_mode: bind_mode.clone(),
        bind_host: bind_host.clone(),
        port,
        token_configured: !token.trim().is_empty(),
        suggested_client_urls: suggested_urls(&bind_host, port),
        log_tail,
        script_found,
        script_path,
    })
}

#[tauri::command]
pub async fn yapper_node_start(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<NodeServerStatus, String> {
    reconcile_child(&state).await;
    {
        let g = state.yapper_node.lock().await;
        if g.is_some() {
            return Err("Processing server is already running.".into());
        }
    }

    let script = yapper_node_main_path(&app);
    if !script.is_file() {
        return Err(format!(
            "Yapper Node script not found: {}. Install or clone the full repo, or set YAPPER_NODE.",
            script.display()
        ));
    }

    let (_bind_mode, port, host, token) = read_settings(&app).await?;
    let token = token.trim();
    if token.is_empty() {
        return Err("Set a server password first (Settings → Network processing server).".into());
    }

    let python = python_executable(&app);
    state.yapper_node_logs.lock().await.clear();

    let mut cmd = Command::new(&python);
    cmd.arg(script.as_os_str())
        .args([
            "--host",
            host.as_str(),
            "--port",
            &port.to_string(),
            "--token",
            token,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .env("PYTHONUNBUFFERED", "1");

    crate::win_spawn::hide_console_tokio_python(&mut cmd, &python);
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to start Yapper Node ({python}): {e}"))?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "node: no stdout".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "node: no stderr".to_string())?;

    let logs = Arc::clone(&state.yapper_node_logs);
    tokio::spawn(async move {
        let mut reader = BufReader::new(stdout).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            push_log(&logs, format!("[stdout] {line}")).await;
        }
    });
    let logs_err = Arc::clone(&state.yapper_node_logs);
    tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            push_log(&logs_err, format!("[stderr] {line}")).await;
        }
    });

    *state.yapper_node.lock().await = Some(child);
    push_log(
        &state.yapper_node_logs,
        format!("Started Yapper Node (Python: {python})"),
    )
    .await;

    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    reconcile_child(&state).await;

    yapper_node_status(app, state).await
}

#[tauri::command]
pub async fn yapper_node_stop(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<NodeServerStatus, String> {
    let mut guard = state.yapper_node.lock().await;
    if let Some(mut child) = guard.take() {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
    drop(guard);
    push_log(&state.yapper_node_logs, "Processing server stopped.".into()).await;
    yapper_node_status(app, state).await
}
