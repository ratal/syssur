//! `syssur-server`: Axum HTTP + WebSocket hub multiplexing per-file Loro CRDT
//! deltas with bidirectional `VersionVector` handshake, SQLite persistence,
//! and pure `.sysml` Git snapshot commits.

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use syssur_collab::{ProjectCollab, SyncMessage};
use syssur_lang::SysmlEngine;
use syssur_store::{ProjectStore, SqliteStore, StoredFileSnapshot};
use tokio::sync::{broadcast, mpsc, Mutex};
use tracing::{info, warn};

struct RoomState {
    collab: Mutex<ProjectCollab>,
    tx: broadcast::Sender<String>,
}

#[derive(Clone)]
struct AppState {
    store: SqliteStore,
    git_root: PathBuf,
    rooms: Arc<Mutex<HashMap<String, Arc<RoomState>>>>,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    service: &'static str,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "syssur_server=info".into()),
        )
        .init();

    let db_url = std::env::var("SYSSUR_DB").unwrap_or_else(|_| "sqlite::memory:".to_string());
    let git_root = PathBuf::from(
        std::env::var("SYSSUR_GIT_DIR").unwrap_or_else(|_| ".syssur-repos".to_string()),
    );
    let store = SqliteStore::connect(&db_url)
        .await
        .expect("connect sqlite store");

    let app = build_router(store, git_root);

    let addr = std::env::var("SYSSUR_ADDR").unwrap_or_else(|_| "127.0.0.1:8090".to_string());
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("bind tcp listener");
    info!("syssur-server listening on http://{addr}");
    axum::serve(listener, app).await.expect("serve axum");
}

fn build_router(store: SqliteStore, git_root: PathBuf) -> Router {
    let state = AppState {
        store,
        git_root,
        rooms: Arc::new(Mutex::new(HashMap::new())),
    };

    Router::new()
        .route("/health", get(health))
        .route("/ws/projects/{project_id}", get(ws_handler))
        .with_state(state)
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        service: "syssur-server",
    })
}

async fn ws_handler(
    Path(project_id): Path<String>,
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(project_id, socket, state))
}

async fn get_or_load_room(state: &AppState, project_id: &str) -> Arc<RoomState> {
    let mut rooms = state.rooms.lock().await;
    if let Some(existing) = rooms.get(project_id) {
        return existing.clone();
    }

    let mut collab = ProjectCollab::default();
    if let Ok(files) = state.store.list_files(project_id).await {
        for snap in files {
            if !snap.crdt_snapshot.is_empty() {
                let _ = collab.import_file_delta(&snap.file_path, &snap.crdt_snapshot);
            } else {
                let _ = collab.open_file(snap.file_path, &snap.content);
            }
        }
    }

    let (tx, _) = broadcast::channel(256);
    let room = Arc::new(RoomState {
        collab: Mutex::new(collab),
        tx,
    });
    rooms.insert(project_id.to_string(), room.clone());
    room
}

async fn handle_socket(project_id: String, socket: WebSocket, state: AppState) {
    let room = get_or_load_room(&state, &project_id).await;
    let mut room_rx = room.tx.subscribe();
    let (mut ws_tx, mut ws_rx) = socket.split();
    let (direct_tx, mut direct_rx) = mpsc::unbounded_channel::<String>();

    // Send server's Hello VersionVector map immediately on connect.
    {
        let collab = room.collab.lock().await;
        let hello = SyncMessage::Hello {
            versions: collab.encode_versions(),
        };
        if let Ok(json) = serde_json::to_string(&hello) {
            let _ = direct_tx.send(json);
        }
    }

    let send_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(msg) = direct_rx.recv() => {
                    if ws_tx.send(Message::Text(msg.into())).await.is_err() {
                        break;
                    }
                }
                Ok(msg) = room_rx.recv() => {
                    if ws_tx.send(Message::Text(msg.into())).await.is_err() {
                        break;
                    }
                }
                else => break,
            }
        }
    });

    while let Some(Ok(Message::Text(text))) = ws_rx.next().await {
        let Ok(msg) = serde_json::from_str::<SyncMessage>(&text) else {
            warn!("ignoring malformed sync message");
            continue;
        };

        match msg {
            SyncMessage::Hello { versions } => {
                let collab = room.collab.lock().await;
                if let Ok(deltas) = collab.compute_missing_deltas(&versions) {
                    for (file, bytes) in deltas {
                        if let Ok(json) = serde_json::to_string(&SyncMessage::Delta { file, bytes })
                        {
                            let _ = direct_tx.send(json);
                        }
                    }
                }
            }
            SyncMessage::Delta { ref file, ref bytes } => {
                let mut collab = room.collab.lock().await;
                if let Err(err) = collab.import_file_delta(file, bytes) {
                    warn!("failed to apply CRDT delta for {file}: {err}");
                    continue;
                }
                if let Some(doc) = collab.files.get(file) {
                    if let Ok(snapshot) = doc.export_snapshot() {
                        let _ = state
                            .store
                            .save_file_snapshot(&StoredFileSnapshot {
                                project_id: project_id.clone(),
                                file_path: file.clone(),
                                content: doc.text(),
                                crdt_snapshot: snapshot,
                            })
                            .await;
                    }
                }
                let parsed = SysmlEngine::parse_workspace(&collab.snapshot_texts());
                let _ = state
                    .store
                    .replace_trace_links(&project_id, &parsed.graph.trace_links)
                    .await;
                let _ = room.tx.send(text.to_string());
            }
            SyncMessage::CommitSnapshot { message } => {
                let repo_dir = state.git_root.join(&project_id);
                match state
                    .store
                    .commit_git_snapshot(&project_id, &repo_dir, &message)
                    .await
                {
                    Ok((commit_id, files)) => {
                        if let Ok(json) = serde_json::to_string(&SyncMessage::CommitResult {
                            commit_id,
                            files,
                        }) {
                            let _ = room.tx.send(json);
                        }
                    }
                    Err(err) => {
                        warn!("git snapshot commit failed for {project_id}: {err}");
                    }
                }
            }
            SyncMessage::SnapshotRequest { file } => {
                let collab = room.collab.lock().await;
                if let Some(doc) = collab.files.get(&file) {
                    if let Ok(bytes) = doc.export_snapshot() {
                        if let Ok(json) =
                            serde_json::to_string(&SyncMessage::Delta { file, bytes })
                        {
                            let _ = direct_tx.send(json);
                        }
                    }
                }
            }
            SyncMessage::Presence(_) | SyncMessage::CommitResult { .. } => {
                let _ = room.tx.send(text.to_string());
            }
        }
    }

    send_task.abort();
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::connect_async;
    use tokio_tungstenite::tungstenite::Message as WsMsg;

    async fn read_sync_msg(
        rx: &mut futures_util::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
        >,
    ) -> SyncMessage {
        while let Some(Ok(WsMsg::Text(txt))) = rx.next().await {
            if let Ok(msg) = serde_json::from_str::<SyncMessage>(&txt) {
                return msg;
            }
        }
        panic!("websocket closed unexpectedly");
    }

    #[tokio::test]
    async fn multi_client_websocket_sync_and_git_snapshot_commit() {
        let store = SqliteStore::connect("sqlite::memory:").await.unwrap();
        let git_root = std::env::temp_dir().join(format!("syssur_ws_git_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&git_root);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = build_router(store, git_root.clone());
        let server_task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let url = format!("ws://{addr}/ws/projects/rover-test");
        let (ws_a, _) = connect_async(&url).await.unwrap();
        let (ws_b, _) = connect_async(&url).await.unwrap();
        let (mut tx_a, mut rx_a) = ws_a.split();
        let (_tx_b, mut rx_b) = ws_b.split();

        // Both clients receive server's initial Hello.
        assert!(matches!(read_sync_msg(&mut rx_a).await, SyncMessage::Hello { .. }));
        assert!(matches!(read_sync_msg(&mut rx_b).await, SyncMessage::Hello { .. }));

        // Client A creates/edits vehicle.sysml and pushes Delta.
        let mut collab_a = ProjectCollab::default();
        collab_a
            .open_file("vehicle.sysml", "package RoverSystem {\n    part def Rover {}\n}\n")
            .unwrap();
        let snap_bytes = collab_a.files["vehicle.sysml"].export_snapshot().unwrap();
        let delta_msg = serde_json::to_string(&SyncMessage::Delta {
            file: "vehicle.sysml".into(),
            bytes: snap_bytes,
        })
        .unwrap();
        tx_a.send(WsMsg::Text(delta_msg.into())).await.unwrap();

        // Client B receives the Delta and converges to identical .sysml text.
        let mut collab_b = ProjectCollab::default();
        let got_b = read_sync_msg(&mut rx_b).await;
        match got_b {
            SyncMessage::Delta { file, bytes } => {
                collab_b.import_file_delta(&file, &bytes).unwrap();
            }
            other => panic!("expected Delta on client B, got {other:?}"),
        }
        assert_eq!(collab_a.snapshot_texts(), collab_b.snapshot_texts());

        // Drain broadcast echo on A, then trigger CommitSnapshot.
        let _ = read_sync_msg(&mut rx_a).await;
        let commit_req = serde_json::to_string(&SyncMessage::CommitSnapshot {
            message: "Multi-client test snapshot".into(),
        })
        .unwrap();
        tx_a.send(WsMsg::Text(commit_req.into())).await.unwrap();

        let commit_res = read_sync_msg(&mut rx_b).await;
        match commit_res {
            SyncMessage::CommitResult { commit_id, files } => {
                assert!(!commit_id.is_empty());
                assert_eq!(files, vec!["vehicle.sysml".to_string()]);
            }
            other => panic!("expected CommitResult, got {other:?}"),
        }

        let committed_file = git_root.join("rover-test").join("vehicle.sysml");
        assert!(committed_file.exists());
        assert_eq!(
            std::fs::read_to_string(&committed_file).unwrap(),
            collab_a.files["vehicle.sysml"].text()
        );
        assert!(git_root.join("rover-test").join(".git").exists());

        server_task.abort();
        let _ = std::fs::remove_dir_all(&git_root);
    }
}
