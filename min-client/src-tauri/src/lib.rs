use std::sync::{Arc, Mutex};

use crypto::Identity;
use network::{ClientEvent, WsHandle};
use storage::{LocalStore, StoredMessage};

mod crypto;
mod network;
mod storage;

struct AppState {
    store: Arc<Mutex<LocalStore>>,
    ws: Mutex<Option<WsHandle>>,
}

#[tauri::command]
fn create_identity(state: tauri::State<AppState>, display_name: String) -> Result<Identity, String> {
    let identity = crypto::generate_identity(display_name);
    state
        .store
        .lock()
        .map_err(|_| "local store poisoned".to_string())?
        .save_identity(&identity)
        .map_err(|err| err.to_string())?;
    Ok(identity)
}

#[tauri::command]
async fn connect_server(
    state: tauri::State<'_, AppState>,
    app_handle: tauri::AppHandle,
    server_url: String,
    user_id: String,
    display_name: String,
    public_key: String,
) -> Result<(), String> {
    network::register(&server_url, &user_id, &display_name, &public_key)
        .await
        .map_err(|err| err.to_string())?;

    let handle = network::connect(&server_url, &user_id, state.store.clone(), app_handle)
        .await
        .map_err(|err| err.to_string())?;

    *state.ws.lock().map_err(|_| "websocket state poisoned".to_string())? = Some(handle);
    Ok(())
}

#[tauri::command]
fn send_message(state: tauri::State<AppState>, recipient_id: String, body: String) -> Result<(), String> {
    let store = state
        .store
        .lock()
        .map_err(|_| "local store poisoned".to_string())?;
    let identity = store
        .latest_identity()
        .map_err(|err| err.to_string())?
        .ok_or_else(|| "create an identity first".to_string())?;
    let key = store
        .conversation_key(&recipient_id)
        .map_err(|err| err.to_string())?;
    let envelope = crypto::encrypt_message(&identity, recipient_id.clone(), &key, &body)
        .map_err(|err| err.to_string())?;

    store
        .save_message(
            &envelope.id.to_string(),
            &recipient_id,
            "outbound",
            &body,
            envelope.created_at,
        )
        .map_err(|err| err.to_string())?;
    drop(store);

    let ws = state
        .ws
        .lock()
        .map_err(|_| "websocket state poisoned".to_string())?
        .clone()
        .ok_or_else(|| "connect to the server first".to_string())?;
    ws.send(ClientEvent::SendMessage {
        to: recipient_id,
        envelope,
    })
    .map_err(|err| err.to_string())
}

#[tauri::command]
fn list_messages(state: tauri::State<AppState>) -> Result<Vec<StoredMessage>, String> {
    state
        .store
        .lock()
        .map_err(|_| "local store poisoned".to_string())?
        .list_messages()
        .map_err(|err| err.to_string())
}

pub fn run() {
    let store = LocalStore::open().expect("failed to open local min database");

    tauri::Builder::default()
        .manage(AppState {
            store: Arc::new(Mutex::new(store)),
            ws: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            create_identity,
            connect_server,
            send_message,
            list_messages
        ])
        .run(tauri::generate_context!())
        .expect("error while running min");
}
