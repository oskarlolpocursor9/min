# min

`min` is a secure desktop messenger prototype built with Tauri, React, Tokio, Axum WebSockets, Noise, Ed25519 identities, AES-GCM message envelopes, PostgreSQL server storage, and SQLite client cache.

## Layout

```text
min/
  min-client/          React + TypeScript UI and Tauri Rust core
  min-server/          Axum message router
  .github/workflows/   Windows release pipeline
```

The large dependency folders in this repository are used as local path dependencies.

## Server

```powershell
$env:DATABASE_URL="postgres://postgres:postgres@localhost/min"
cargo run -p min-server
```

## Client

```powershell
cd min-client
npm.cmd install
npm.cmd run tauri dev
```

Rust, Cargo, Node, and PostgreSQL are required locally.
