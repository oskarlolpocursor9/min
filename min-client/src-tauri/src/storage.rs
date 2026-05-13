use std::path::PathBuf;

use chrono::{DateTime, Utc};
use rusqlite::{params, Connection};
use serde::Serialize;

use crate::crypto::Identity;

#[derive(Debug, Serialize)]
pub struct StoredMessage {
    pub id: String,
    pub peer_id: String,
    pub direction: String,
    pub body: String,
    pub created_at: DateTime<Utc>,
}

pub struct LocalStore {
    conn: Connection,
}

impl LocalStore {
    pub fn open() -> anyhow::Result<Self> {
        let path = database_path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(path)?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    pub fn save_identity(&self, identity: &Identity) -> anyhow::Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO identities (id, display_name, public_key, secret_key)
            VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(id) DO UPDATE SET
                display_name = excluded.display_name,
                public_key = excluded.public_key,
                secret_key = excluded.secret_key
            "#,
            params![identity.id, identity.display_name, identity.public_key, identity.secret_key],
        )?;
        Ok(())
    }

    pub fn latest_identity(&self) -> anyhow::Result<Option<Identity>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, display_name, public_key, secret_key FROM identities ORDER BY created_at DESC LIMIT 1",
        )?;
        let mut rows = stmt.query([])?;
        let Some(row) = rows.next()? else {
            return Ok(None);
        };

        Ok(Some(Identity {
            id: row.get(0)?,
            display_name: row.get(1)?,
            public_key: row.get(2)?,
            secret_key: row.get(3)?,
        }))
    }

    pub fn save_message(
        &self,
        id: &str,
        peer_id: &str,
        direction: &str,
        body: &str,
        created_at: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        self.conn.execute(
            r#"
            INSERT INTO messages (id, peer_id, direction, body, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(id) DO NOTHING
            "#,
            params![id, peer_id, direction, body, created_at.to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn conversation_key(&self, peer_id: &str) -> anyhow::Result<String> {
        let mut stmt = self
            .conn
            .prepare("SELECT conversation_key FROM conversations WHERE peer_id = ?1")?;
        let existing = stmt.query_row([peer_id], |row| row.get::<_, String>(0)).ok();

        if let Some(key) = existing {
            return Ok(key);
        }

        let key = crate::crypto::new_conversation_key();
        self.conn.execute(
            "INSERT INTO conversations (peer_id, conversation_key) VALUES (?1, ?2)",
            params![peer_id, key],
        )?;
        Ok(key)
    }

    pub fn list_messages(&self) -> anyhow::Result<Vec<StoredMessage>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, peer_id, direction, body, created_at FROM messages ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            let created_at: String = row.get(4)?;
            Ok(StoredMessage {
                id: row.get(0)?,
                peer_id: row.get(1)?,
                direction: row.get(2)?,
                body: row.get(3)?,
                created_at: DateTime::parse_from_rfc3339(&created_at)
                    .map(|dt| dt.with_timezone(&Utc))
                    .unwrap_or_else(|_| Utc::now()),
            })
        })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    fn migrate(&self) -> anyhow::Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS identities (
                id TEXT PRIMARY KEY,
                display_name TEXT NOT NULL,
                public_key TEXT NOT NULL,
                secret_key TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS conversations (
                peer_id TEXT PRIMARY KEY,
                conversation_key TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                peer_id TEXT NOT NULL,
                direction TEXT NOT NULL CHECK(direction IN ('inbound', 'outbound')),
                body TEXT NOT NULL,
                created_at TEXT NOT NULL
            );
            "#,
        )?;
        Ok(())
    }
}

fn database_path() -> anyhow::Result<PathBuf> {
    let mut dir = dirs::data_local_dir().ok_or_else(|| anyhow::anyhow!("missing local data dir"))?;
    dir.push("min");
    dir.push("min.sqlite");
    Ok(dir)
}
