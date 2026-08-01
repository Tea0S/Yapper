//! Remote Yapper Node bridge with reconnect.

use crate::sidecar::{SidecarIn, SidecarOut};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::{connect_async, tungstenite::Message};

pub struct RemoteBridge {
    pub tx: mpsc::UnboundedSender<SidecarIn>,
    pub pending: Arc<Mutex<VecDeque<SidecarOut>>>,
    pub alive: Arc<AtomicBool>,
}

pub async fn spawn_remote(url: &str, token: &str) -> Result<RemoteBridge, String> {
    let url = url.to_string();
    let token = token.to_string();
    let pending = Arc::new(Mutex::new(VecDeque::new()));
    let alive = Arc::new(AtomicBool::new(true));
    let (tx, mut rx) = mpsc::unbounded_channel::<SidecarIn>();

    let pending_reader = Arc::clone(&pending);
    let alive_flag = Arc::clone(&alive);

    tokio::spawn(async move {
        let mut backoff_ms: u64 = 500;
        let mut last_init: Option<SidecarIn> = None;

        'reconnect: loop {
            alive_flag.store(true, Ordering::SeqCst);
            let connect_result = connect_async(&url).await;
            let (ws, _) = match connect_result {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("[yapper-remote] connect failed: {e}");
                    alive_flag.store(false, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                    backoff_ms = (backoff_ms * 2).min(30_000);
                    // Drain shutdown if requested while disconnected.
                    while let Ok(cmd) = rx.try_recv() {
                        if matches!(cmd, SidecarIn::Shutdown) {
                            return;
                        }
                        if matches!(cmd, SidecarIn::Init { .. }) {
                            last_init = Some(cmd);
                        }
                    }
                    continue 'reconnect;
                }
            };
            backoff_ms = 500;
            let (mut write, mut read) = ws.split();

            let hello = json!({ "type": "auth", "token": token }).to_string();
            if write.send(Message::Text(hello.into())).await.is_err() {
                alive_flag.store(false, Ordering::SeqCst);
                tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
                continue 'reconnect;
            }

            // Re-init after reconnect so the node has model config.
            if let Some(init) = last_init.clone() {
                if let Ok(line) = serde_json::to_string(&init) {
                    let _ = write.send(Message::Text(line.into())).await;
                }
            }

            loop {
                tokio::select! {
                    cmd = rx.recv() => {
                        match cmd {
                            None => {
                                alive_flag.store(false, Ordering::SeqCst);
                                return;
                            }
                            Some(SidecarIn::Shutdown) => {
                                let _ = write.send(Message::Close(None)).await;
                                alive_flag.store(false, Ordering::SeqCst);
                                return;
                            }
                            Some(other) => {
                                if matches!(other, SidecarIn::Init { .. }) {
                                    last_init = Some(other.clone());
                                }
                                if let Ok(line) = serde_json::to_string(&other) {
                                    if write.send(Message::Text(line.into())).await.is_err() {
                                        alive_flag.store(false, Ordering::SeqCst);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    msg = read.next() => {
                        match msg {
                            Some(Ok(Message::Text(t))) => {
                                if let Ok(m) = serde_json::from_str::<SidecarOut>(&t) {
                                    let mut q = pending_reader.lock().await;
                                    const PENDING_CAP: usize = 256;
                                    while q.len() >= PENDING_CAP {
                                        let _ = q.pop_front();
                                    }
                                    q.push_back(m);
                                }
                            }
                            Some(Ok(Message::Close(_))) | None => {
                                alive_flag.store(false, Ordering::SeqCst);
                                break;
                            }
                            Some(Err(_)) => {
                                alive_flag.store(false, Ordering::SeqCst);
                                break;
                            }
                            _ => {}
                        }
                    }
                }
            }

            eprintln!("[yapper-remote] connection lost — reconnecting…");
            tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            backoff_ms = (backoff_ms * 2).min(30_000);
        }
    });

    Ok(RemoteBridge { tx, pending, alive })
}
