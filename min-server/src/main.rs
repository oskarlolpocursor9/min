use std::{collections::HashMap, net::SocketAddr, sync::Arc};

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Path, State,
    },
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tokio::sync::{mpsc, RwLock};
use tracing::{error, info, warn};
use uuid::Uuid;

type Clients = Arc<RwLock<HashMap<String, mpsc::UnboundedSender<ServerEvent>>>>;

#[derive(Clone)]
struct AppState {
    db: PgPool,
    clients: Clients,
}

#[derive(Debug, Deserialize)]
struct RegisterRequest {
    id: String,
    display_name: String,
    public_key: String,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct UserRecord {
    id: String,
    display_name: String,
    public_key: String,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct EncryptedEnvelope {
    id: Uuid,
    nonce: String,
    ciphertext: String,
    sender_public_key: String,
    recipient_id: String,
    created_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientEvent {
    SendMessage { to: String, envelope: EncryptedEnvelope },
    Ping,
}

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerEvent {
    Delivery { from: String, envelope: EncryptedEnvelope },
    Ack { id: Uuid },
    Error { message: String },
    Pong,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let database_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://postgres:postgres@localhost/min".to_string());
    let db = PgPool::connect(&database_url).await?;
    sqlx::migrate!("./migrations").run(&db).await?;

    let state = AppState {
        db,
        clients: Arc::new(RwLock::new(HashMap::new())),
    };

    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/register", post(register))
        .route("/login", post(login))
        .route("/users/{id}/key", get(user_key))
        .route("/ws/{user_id}", get(ws_handler))
        .with_state(state);

    let addr: SocketAddr = "0.0.0.0:3027".parse()?;
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("min-server listening on ws://{addr}/ws/:user_id");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn register(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<UserRecord>, ApiError> {
    let user = sqlx::query_as::<_, UserRecord>(
        r#"
        INSERT INTO users (id, display_name, public_key)
        VALUES ($1, $2, $3)
        ON CONFLICT (id) DO UPDATE
        SET display_name = EXCLUDED.display_name,
            public_key = EXCLUDED.public_key
        RETURNING id, display_name, public_key, created_at
        "#,
    )
    .bind(req.id)
    .bind(req.display_name)
    .bind(req.public_key)
    .fetch_one(&state.db)
    .await?;

    Ok(Json(user))
}

async fn login(
    State(state): State<AppState>,
    Json(req): Json<RegisterRequest>,
) -> Result<Json<UserRecord>, ApiError> {
    register(State(state), Json(req)).await
}

async fn user_key(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<UserRecord>, ApiError> {
    let user = sqlx::query_as::<_, UserRecord>(
        "SELECT id, display_name, public_key, created_at FROM users WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or(ApiError::NotFound)?;

    Ok(Json(user))
}

async fn ws_handler(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(state, user_id, socket))
}

async fn handle_socket(state: AppState, user_id: String, socket: WebSocket) {
    let (mut sender, mut receiver) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerEvent>();

    state.clients.write().await.insert(user_id.clone(), tx);
    info!(%user_id, "client connected");

    let writer = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            match serde_json::to_string(&event) {
                Ok(text) => {
                    if sender.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                Err(err) => error!(?err, "failed to encode server event"),
            }
        }
    });

    while let Some(result) = receiver.next().await {
        match result {
            Ok(Message::Text(text)) => match serde_json::from_str::<ClientEvent>(&text) {
                Ok(event) => handle_event(&state, &user_id, event).await,
                Err(err) => warn!(?err, %user_id, "invalid client event"),
            },
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(err) => {
                warn!(?err, %user_id, "websocket error");
                break;
            }
        }
    }

    state.clients.write().await.remove(&user_id);
    writer.abort();
    info!(%user_id, "client disconnected");
}

async fn handle_event(state: &AppState, sender_id: &str, event: ClientEvent) {
    match event {
        ClientEvent::Ping => {
            if let Some(tx) = state.clients.read().await.get(sender_id) {
                let _ = tx.send(ServerEvent::Pong);
            }
        }
        ClientEvent::SendMessage { to, envelope } => {
            let envelope_json = match serde_json::to_value(&envelope) {
                Ok(value) => value,
                Err(err) => {
                    error!(?err, "failed to serialize envelope");
                    return;
                }
            };

            if let Err(err) = sqlx::query(
                r#"
                INSERT INTO queued_messages (id, sender_id, recipient_id, envelope)
                VALUES ($1, $2, $3, $4)
                ON CONFLICT (id) DO NOTHING
                "#,
            )
            .bind(envelope.id)
            .bind(sender_id)
            .bind(&to)
            .bind(envelope_json)
            .execute(&state.db)
            .await
            {
                error!(?err, "failed to store message");
                return;
            }

            if let Some(tx) = state.clients.read().await.get(&to) {
                let _ = tx.send(ServerEvent::Delivery {
                    from: sender_id.to_string(),
                    envelope: envelope.clone(),
                });
                let _ = sqlx::query("UPDATE queued_messages SET delivered_at = now() WHERE id = $1")
                    .bind(envelope.id)
                .execute(&state.db)
                .await;
            }

            if let Some(tx) = state.clients.read().await.get(sender_id) {
                let _ = tx.send(ServerEvent::Ack { id: envelope.id });
            }
        }
    }
}

enum ApiError {
    Db(sqlx::Error),
    NotFound,
}

impl From<sqlx::Error> for ApiError {
    fn from(value: sqlx::Error) -> Self {
        Self::Db(value)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        match self {
            ApiError::Db(err) => {
                error!(?err, "database error");
                (StatusCode::INTERNAL_SERVER_ERROR, "database error").into_response()
            }
            ApiError::NotFound => (StatusCode::NOT_FOUND, "not found").into_response(),
        }
    }
}
