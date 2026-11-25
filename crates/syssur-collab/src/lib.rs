//! Per-file Loro CRDT collaboration engine for canonical `.sysml` documents
//! and user presence (cursor/selection awareness). Works identically on Native and WASM.

use loro::{ExportMode, LoroDoc, LoroEncodeError, LoroError, UpdateOptions, VersionVector};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

const TEXT_CONTAINER: &str = "sysml";

#[derive(Debug, Error)]
pub enum CollabError {
    #[error("loro error: {0}")]
    Loro(#[from] LoroError),
    #[error("loro encode error: {0}")]
    Encode(#[from] LoroEncodeError),
    #[error("loro text diff timeout")]
    UpdateTimeout,
    #[error("unknown file '{0}'")]
    UnknownFile(String),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserPresence {
    pub user_id: String,
    pub display_name: String,
    pub active_file: Option<String>,
    pub selected_instance: Option<String>,
}

/// Wire envelope multiplexed over the project WebSocket room.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncMessage {
    /// Bidirectional handshake carrying encoded `VersionVector` per file.
    Hello {
        versions: BTreeMap<String, Vec<u8>>,
    },
    Delta {
        file: String,
        bytes: Vec<u8>,
    },
    SnapshotRequest {
        file: String,
    },
    CommitSnapshot {
        message: String,
    },
    CommitResult {
        commit_id: String,
        files: Vec<String>,
    },
    Presence(UserPresence),
}

pub struct FileDoc {
    doc: LoroDoc,
}

impl FileDoc {
    pub fn new_with_text(initial: &str) -> Result<Self, CollabError> {
        let doc = LoroDoc::new();
        let text = doc.get_text(TEXT_CONTAINER);
        text.insert(0, initial)?;
        doc.commit();
        Ok(Self { doc })
    }

    pub fn from_snapshot(bytes: &[u8]) -> Result<Self, CollabError> {
        let doc = LoroDoc::new();
        doc.import(bytes)?;
        Ok(Self { doc })
    }

    pub fn text(&self) -> String {
        self.doc.get_text(TEXT_CONTAINER).to_string()
    }

    /// Applies a full-text diff update (e.g. from the code editor or command engine)
    /// and returns a binary delta if anything changed.
    pub fn set_text(&self, new_text: &str) -> Result<Option<Vec<u8>>, CollabError> {
        let before_vv = self.doc.oplog_vv();
        let text = self.doc.get_text(TEXT_CONTAINER);
        if text.to_string() == new_text {
            return Ok(None);
        }
        text.update(new_text, UpdateOptions::default())
            .map_err(|_| CollabError::UpdateTimeout)?;
        self.doc.commit();
        let delta = self.doc.export(ExportMode::updates(&before_vv))?;
        Ok(Some(delta))
    }

    /// Applies a byte-range splice (`start..end`) and returns a binary delta.
    pub fn apply_byte_edit(
        &self,
        start_byte: usize,
        end_byte: usize,
        replacement: &str,
    ) -> Result<Vec<u8>, CollabError> {
        let before_vv = self.doc.oplog_vv();
        let current = self.text();
        let char_start = current[..start_byte.min(current.len())].chars().count();
        let char_end = current[..end_byte.min(current.len())].chars().count();
        let char_len = char_end.saturating_sub(char_start);
        let text = self.doc.get_text(TEXT_CONTAINER);
        text.splice(char_start, char_len, replacement)?;
        self.doc.commit();
        Ok(self.doc.export(ExportMode::updates(&before_vv))?)
    }

    pub fn export_snapshot(&self) -> Result<Vec<u8>, CollabError> {
        Ok(self.doc.export(ExportMode::Snapshot)?)
    }

    pub fn export_updates_since(&self, vv: &VersionVector) -> Result<Vec<u8>, CollabError> {
        Ok(self.doc.export(ExportMode::updates(vv))?)
    }

    pub fn export_updates_since_bytes(&self, vv_bytes: &[u8]) -> Result<Vec<u8>, CollabError> {
        let vv = VersionVector::decode(vv_bytes)?;
        self.export_updates_since(&vv)
    }

    pub fn import_delta(&self, bytes: &[u8]) -> Result<(), CollabError> {
        self.doc.import(bytes)?;
        Ok(())
    }

    pub fn version(&self) -> VersionVector {
        self.doc.oplog_vv()
    }

    pub fn encode_version(&self) -> Vec<u8> {
        self.doc.oplog_vv().encode()
    }
}

/// Manages one Loro CRDT document per `.sysml` file in a workspace/project.
#[derive(Default)]
pub struct ProjectCollab {
    pub files: BTreeMap<String, FileDoc>,
    pub presences: BTreeMap<String, UserPresence>,
}

impl ProjectCollab {
    pub fn open_file(&mut self, path: impl Into<String>, initial_text: &str) -> Result<(), CollabError> {
        let path = path.into();
        let doc = FileDoc::new_with_text(initial_text)?;
        self.files.insert(path, doc);
        Ok(())
    }

    pub fn snapshot_texts(&self) -> BTreeMap<String, String> {
        self.files
            .iter()
            .map(|(k, v)| (k.clone(), v.text()))
            .collect()
    }

    pub fn encode_versions(&self) -> BTreeMap<String, Vec<u8>> {
        self.files
            .iter()
            .map(|(k, doc)| (k.clone(), doc.encode_version()))
            .collect()
    }

    /// Computes minimal binary deltas needed by a peer given its `VersionVector` map.
    pub fn compute_missing_deltas(
        &self,
        remote_versions: &BTreeMap<String, Vec<u8>>,
    ) -> Result<Vec<(String, Vec<u8>)>, CollabError> {
        let mut deltas = Vec::new();
        for (file, doc) in &self.files {
            if let Some(vv_bytes) = remote_versions.get(file) {
                let remote_vv = VersionVector::decode(vv_bytes)?;
                if doc.version() != remote_vv {
                    let bytes = doc.export_updates_since(&remote_vv)?;
                    if !bytes.is_empty() {
                        deltas.push((file.clone(), bytes));
                    }
                }
            } else {
                deltas.push((file.clone(), doc.export_snapshot()?));
            }
        }
        Ok(deltas)
    }

    pub fn set_file_text(&self, file: &str, new_text: &str) -> Result<Option<Vec<u8>>, CollabError> {
        let doc = self
            .files
            .get(file)
            .ok_or_else(|| CollabError::UnknownFile(file.to_string()))?;
        doc.set_text(new_text)
    }

    pub fn import_file_delta(&mut self, file: &str, bytes: &[u8]) -> Result<(), CollabError> {
        if let Some(doc) = self.files.get(file) {
            doc.import_delta(bytes)
        } else {
            let doc = FileDoc::from_snapshot(bytes)?;
            self.files.insert(file.to_string(), doc);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_crdt_edits_converge_identically() {
        let initial = "package Power {\n    part def BatteryPack {}\n}\n";
        let peer_a = FileDoc::new_with_text(initial).unwrap();
        let snap = peer_a.export_snapshot().unwrap();
        let peer_b = FileDoc::from_snapshot(&snap).unwrap();

        let vv_a = peer_a.version();
        let vv_b = peer_b.version();

        // Peer A inserts at end; Peer B inserts inside BatteryPack concurrently.
        let end = initial.len();
        peer_a.apply_byte_edit(end, end, "// end comment\n").unwrap();
        let insert_pos = initial.find("{}").unwrap() + 1;
        peer_b
            .apply_byte_edit(insert_pos, insert_pos, "\n        port powerOut : PowerPort;\n    ")
            .unwrap();

        let delta_a = peer_a.export_updates_since(&vv_a).unwrap();
        let delta_b = peer_b.export_updates_since(&vv_b).unwrap();

        peer_a.import_delta(&delta_b).unwrap();
        peer_b.import_delta(&delta_a).unwrap();

        assert_eq!(peer_a.text(), peer_b.text());
        assert!(peer_a.text().contains("port powerOut : PowerPort;"));
        assert!(peer_a.text().contains("// end comment"));
    }

    #[test]
    fn hello_handshake_reconciles_offline_edits_across_projects() {
        let mut client = ProjectCollab::default();
        client.open_file("power.sysml", "package Power {}\n").unwrap();
        let snap = client.files["power.sysml"].export_snapshot().unwrap();

        let mut server = ProjectCollab::default();
        server.import_file_delta("power.sysml", &snap).unwrap();

        // Both edit offline while disconnected: client adds a comment, server adds a new file.
        client
            .set_file_text("power.sysml", "package Power {\n    // client offline\n}\n")
            .unwrap();
        server
            .open_file("views.sysml", "package RoverViews {}\n")
            .unwrap();

        let client_vv = client.encode_versions();
        let server_vv = server.encode_versions();

        for (file, delta) in server.compute_missing_deltas(&client_vv).unwrap() {
            client.import_file_delta(&file, &delta).unwrap();
        }
        for (file, delta) in client.compute_missing_deltas(&server_vv).unwrap() {
            server.import_file_delta(&file, &delta).unwrap();
        }

        assert_eq!(client.snapshot_texts(), server.snapshot_texts());
        assert!(server.files["power.sysml"].text().contains("client offline"));
        assert!(client.files.contains_key("views.sysml"));
    }
}
