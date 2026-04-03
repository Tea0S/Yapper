use crate::sidecar::{SidecarIn, SidecarOut};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tokio_tungstenite::{connect_async, tungstenite::Message};

pub struct RemoteBridge {
    pub tx: mpsc::UnboundedSender<SidecarIn>,
    pub pending: Arc<Mutex<VecDeque<SidecarOut>>>,
}

pub async fn spawn_remote(url: &str, token: &str) -> Result<RemoteBridge, String> {
    let (ws, _) = connect_async(url).await.map_err(|e| e.to_string())?;
    let (mut write, mut read) = ws.split();

    let hello = json!({ "type": "auth", "token": token }).to_string();
    write
        .send(Message::Text(hello.into()))
        .await
        .map_err(|e| e.to_string())?;

    let pending = Arc::new(Mutex::new(VecDeque::new()));
    let pending_reader = Arc::clone(&pending);
    let (tx, mut rx) = mpsc::unbounded_channel::<SidecarIn>();

    tokio::spawn(async move {
        loop {
            tokio::select! {
                cmd = rx.recv() => {
                    match cmd {
                        None => break,
                        Some(SidecarIn::Shutdown) => {
                            let _ = write.send(Message::Close(None)).await;
                            break;
                        }
                        Some(other) => {
                            if let Ok(line) = serde_json::to_string(&other) {
                                if write.send(Message::Text(line.into())).await.is_err() {
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
                                q.push_back(m);
                            }
                        }
                        Some(Ok(Message::Close(_))) | None => break,
                        Some(Err(_)) => break,
                        _ => {}
                    }
                }
            }
        }
    });

    Ok(RemoteBridge { tx, pending })
}
