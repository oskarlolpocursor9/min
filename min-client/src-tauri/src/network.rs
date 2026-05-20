use std::sync::{Arc, Mutex};

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tauri::Emitter;
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use uuid::Uuid;

use crate::{
    crypto::{self, EncryptedEnvelope},
    storage::LocalStore,
};

#[derive(Debug, Serialize)]
struct RegisterRequest {
    id: String,
    display_name: String,
    public_key: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientEvent {
    SendMessage { to: String, envelope: EncryptedEnvelope },
    Ping,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerEvent {
    Delivery { from: String, envelope: EncryptedEnvelope },
    Ack { id: Uuid },
    Error { message: String },
    Pong,
}

#[derive(Clone)]
pub struct WsHandle {
    tx: mpsc::UnboundedSender<ClientEvent>,
}

impl WsHandle {
    pub fn send(&self, event: ClientEvent) -> anyhow::Result<()> {
        self.tx
            .send(event)
            .map_err(|_| anyhow::anyhow!("websocket writer is closed"))
    }
}

pub async fn register(server_ws_url: &str, id: &str, display_name: &str, public_key: &str) -> anyhow::Result<()> {
    let base = server_ws_url
        .trim_end_matches('/')
        .replace("ws://", "http://")
        .replace("wss://", "https://");
    let http_base = base.strip_suffix("/ws").unwrap_or(&base);

    reqwest::Client::new()
        .post(format!("{http_base}/register"))
        .json(&RegisterRequest {
            id: id.to_string(),
            display_name: display_name.to_string(),
            public_key: public_key.to_string(),
        })
        .send()
        .await?
        .error_for_status()?;

    Ok(())
}

pub async fn connect(
    server_ws_url: &str,
    user_id: &str,
    store: Arc<Mutex<LocalStore>>,
    app: tauri::AppHandle,
) -> anyhow::Result<WsHandle> {
    let _noise = crypto::build_noise_initiator()?;
    let url = format!("{}/{}", server_ws_url.trim_end_matches('/'), user_id);
    let (ws, _) = connect_async(url).await?;
    let (mut writer, mut reader) = ws.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<ClientEvent>();

    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            let Ok(text) = serde_json::to_string(&event) else {
                continue;
            };
            if writer.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    tokio::spawn(async move {
        while let Some(result) = reader.next().await {
            let Ok(Message::Text(text)) = result else {
                continue;
            };
            let Ok(event) = serde_json::from_str::<ServerEvent>(&text) else {
                continue;
            };
            if let ServerEvent::Delivery { from, envelope } = event {
                let body = {
                    let store = store.lock().expect("local store poisoned");
                    match store
                        .conversation_key(&from)
                        .and_then(|key| crypto::decrypt_message(&envelope, &key))
                    {
                        Ok(body) => body,
                        Err(_) => "[encrypted message]".to_string(),
                    }
                };

                let store = store.lock().expect("local store poisoned");
                let _ = store.save_message(
                    &envelope.id.to_string(),
                    &from,
                    "inbound",
                    &body,
                    envelope.created_at,
                );

                let _ = app.emit("new-message", ());
            }
        }
    });

    Ok(WsHandle { tx })
}
