use std::fmt;
use std::hash::Hash;
use std::sync::{Arc, Mutex, Weak};

use crate::node::topic::{SubscribableTopic, Subscription};
use anyhow::Result;
use chrono::{DateTime, Utc};
use hex::FromHexError;
use loro::{ExportMode, LoroDoc, LoroText, UndoManager, event::Diff};
use p2panda_core;
use p2panda_core::cbor::{decode_cbor, encode_cbor};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;
use tracing::error;

use super::author::Author;
use super::authors::Authors;
use super::identity::PublicKey;
use super::service::Service;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DocumentId([u8; 32]);

impl From<DocumentId> for [u8; 32] {
    fn from(id: DocumentId) -> Self {
        id.0
    }
}

impl From<[u8; 32]> for DocumentId {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl fmt::Display for DocumentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl DocumentId {
    pub fn new() -> Self {
        let mut arr = [0u8; 32];
        rand::rng().fill_bytes(&mut arr);
        DocumentId(arr)
    }

    pub fn from_hex(hex: &str) -> Result<DocumentId, FromHexError> {
        let mut bytes = [0u8; 32];
        hex::decode_to_slice(hex, &mut bytes as &mut [u8])?;

        Ok(DocumentId(bytes))
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub enum EphemeralData {
    Cursor {
        insert_cursor: Option<loro::cursor::Cursor>,
        selection_bound: Option<loro::cursor::Cursor>,
        timestamp: std::time::SystemTime,
    },
}

struct DocumentInner {
    id: DocumentId,
    loro_doc: LoroDoc,
    authors: Authors,
    subscription: Mutex<Option<Arc<Subscription<DocumentHandle>>>>,
    service: Service,
    insert_cursor: Mutex<Option<loro::cursor::Cursor>>,
    selection_bound: Mutex<Option<loro::cursor::Cursor>>,
    last_accessed: Mutex<Option<DateTime<Utc>>>,
    background_tasks: Mutex<Vec<JoinHandle<()>>>,
}

#[derive(Clone)]
pub struct Document(Arc<DocumentInner>);

impl Document {
    pub(crate) fn new(service: &Service, id: &DocumentId) -> Self {
        let loro_doc = LoroDoc::new();
        let mut undo_manager = UndoManager::new(&loro_doc);
        undo_manager.set_merge_interval(1000);

        let authors = Authors::new();
        authors.add_this_device(service.private_key().public_key(), None);

        let inner = DocumentInner {
            id: *id,
            loro_doc,
            authors,
            subscription: Mutex::new(None),
            service: service.clone(),
            insert_cursor: Mutex::new(None),
            selection_bound: Mutex::new(None),
            last_accessed: Mutex::new(None),
            background_tasks: Mutex::new(Vec::new()),
        };

        let doc = Document(Arc::new(inner));
        doc.setup_loro_peer_id();
        doc.setup_loro_subscriptions();
        doc
    }

    fn setup_loro_peer_id(&self) {
        let public_key = self.service().private_key().public_key();
        if let Err(err) = self.0.loro_doc.set_peer_id({
            let mut buf = [0u8; 8];
            buf[..8].copy_from_slice(&public_key.0.as_bytes()[..8]);
            u64::from_be_bytes(buf)
        }) {
            error!("set peer id failed: {err}");
        }
    }

    fn setup_loro_subscriptions(&self) {
        let text_id = loro::ContainerID::new_root("document", loro::ContainerType::Text);
        let obj = self.clone();

        self.0
            .loro_doc
            .subscribe(
                &text_id,
                Arc::new(move |loro_event| {
                    let text_deltas = loro_event.events.into_iter().filter_map(|event| {
                        if event.is_unknown {
                            return None;
                        }

                        if let Diff::Text(loro_deltas) = event.diff {
                            Some(loro_deltas)
                        } else {
                            None
                        }
                    });

                    for commit in text_deltas {
                        let mut index = 0;
                        for delta in commit {
                            match delta {
                                loro::TextDelta::Retain { retain, .. } => {
                                    index += retain;
                                }
                                loro::TextDelta::Insert { insert, .. } => {
                                    let len = insert.len();
                                    obj.on_text_inserted(index as i32, insert);
                                    index += len;
                                }
                                loro::TextDelta::Delete { delete } => {
                                    obj.on_range_deleted(index as i32, (index + delete) as i32);
                                }
                            }
                        }
                    }
                }),
            )
            .detach();

        let obj = self.clone();
        self.0
            .loro_doc
            .subscribe_local_update(Box::new(move |delta_bytes| {
                let delta_bytes = delta_bytes.to_vec();
                obj.mark_for_snapshot();

                if let Some(subscription) = obj.subscription() {
                    let subscription = subscription.clone();
                    let handle = tokio::spawn(async move {
                        if let Err(error) = subscription.send_delta(delta_bytes).await {
                            error!("Failed to send delta of document to the network: {error}");
                        }
                    });
                    obj.0.background_tasks.lock().unwrap().push(handle);
                }

                true
            }))
            .detach();
    }

    pub fn insert_text(&self, pos: i32, text: &str) -> Result<()> {
        let text_id = loro::ContainerID::new_root("document", loro::ContainerType::Text);
        let doc = &self.0.loro_doc;
        let text_node: LoroText = doc.get_text(&text_id);
        text_node.insert(pos as usize, text)?;
        doc.commit();
        Ok(())
    }

    pub fn delete_range(&self, start_pos: i32, end_pos: i32) -> Result<()> {
        let text_id = loro::ContainerID::new_root("document", loro::ContainerType::Text);
        let doc = &self.0.loro_doc;
        let text_node: LoroText = doc.get_text(&text_id);
        text_node.delete(start_pos as usize, (end_pos - start_pos) as usize)?;
        doc.commit();
        Ok(())
    }

    pub fn text(&self) -> String {
        let text_id = loro::ContainerID::new_root("document", loro::ContainerType::Text);
        self.0.loro_doc.get_text(&text_id).to_string()
    }

    pub fn id(&self) -> DocumentId {
        self.0.id
    }

    pub fn authors(&self) -> &Authors {
        &self.0.authors
    }

    pub async fn subscribe(&self) {
        if self.subscribed() {
            return;
        }

        let handle = DocumentHandle(Arc::downgrade(&self.0));
        match self.service().node().subscribe(self.id(), handle).await {
            Ok(subscription) => {
                *self.0.subscription.lock().unwrap() = Some(Arc::new(subscription));
            }
            Err(error) => {
                error!("Failed to subscribe to document: {}", error);
            }
        }

        *self.0.last_accessed.lock().unwrap() = None;
        self.store_snapshot().await;
    }

    pub async fn unsubscribe(&self) {
        let subscription = self.0.subscription.lock().unwrap().take();

        if let Some(subscription_arc) = subscription {
            let snapshot_bytes = self
                .0
                .loro_doc
                .export(ExportMode::Snapshot)
                .expect("encoded crdt snapshot");

            if let Err(error) = subscription_arc.send_snapshot(snapshot_bytes).await {
                error!(
                    "Failed to send snapshot of document to the network: {}",
                    error
                );
            }

            let tasks = {
                let mut tasks = self.0.background_tasks.lock().unwrap();
                std::mem::take(&mut *tasks)
            };

            for task in tasks {
                if let Err(error) = task.await {
                    error!("Failed to complete task while unsubscribing: {error}");
                }
            }

            if let Ok(subscription) = Arc::try_unwrap(subscription_arc) {
                if let Err(error) = subscription.unsubscribe().await {
                    error!("Failed to unsubscribe document: {}", error);
                }
            } else {
                error!("Subscription still shared, skip unsubscribe");
            }
        }

        *self.0.last_accessed.lock().unwrap() = Some(Utc::now());
    }

    pub(crate) async fn store_snapshot(&self) {
        if let Some(subscription) = self.subscription() {
            let snapshot_bytes = self
                .0
                .loro_doc
                .export(ExportMode::Snapshot)
                .expect("encoded crdt snapshot");
            if let Err(error) = subscription.send_snapshot(snapshot_bytes).await {
                error!(
                    "Failed to send snapshot of document to the network: {}",
                    error
                );
            }
        }
    }

    pub(crate) fn load_authors(&self, authors: Vec<Author>) {
        self.0.authors.load(authors);
    }

    pub(crate) fn set_last_accessed(&self, ts: Option<DateTime<Utc>>) {
        *self.0.last_accessed.lock().unwrap() = ts;
    }

    fn on_remote_message(&self, bytes: Vec<u8>) {
        if let Err(err) = self.0.loro_doc.import_with(&bytes, "delta") {
            error!("received invalid message: {}", err);
        }
    }

    fn subscription(&self) -> Option<Arc<Subscription<DocumentHandle>>> {
        self.0.subscription.lock().unwrap().clone()
    }

    fn subscribed(&self) -> bool {
        self.subscription().is_some()
    }

    fn service(&self) -> &Service {
        &self.0.service
    }

    fn broadcast_ephemeral(&self) {
        let cursor_data = EphemeralData::Cursor {
            insert_cursor: self.0.insert_cursor.lock().unwrap().clone(),
            selection_bound: self.0.selection_bound.lock().unwrap().clone(),
            timestamp: std::time::SystemTime::now(),
        };

        let cursor_bytes = match encode_cbor(&cursor_data) {
            Ok(data) => data,
            Err(error) => {
                error!("Failed to serialize cursor: {}", error);
                return;
            }
        };

        if let Some(subscription) = self.subscription() {
            let subscription = subscription.clone();
            let handle = tokio::spawn(async move {
                if let Err(error) = subscription.send_ephemeral(cursor_bytes).await {
                    error!("Failed to send cursor position: {}", error);
                }
            });
            self.0.background_tasks.lock().unwrap().push(handle);
        }
    }

    fn on_text_inserted(&self, _pos: i32, _text: String) {
        // TUI pulls text periodically; no-op hook
    }

    fn on_range_deleted(&self, _start: i32, _end: i32) {
        // TUI pulls text periodically; no-op hook
    }

    fn mark_for_snapshot(&self) {
        let obj = self.clone();
        let handle = tokio::spawn(async move {
            obj.store_snapshot().await;
        });
        self.0.background_tasks.lock().unwrap().push(handle);
    }
}

unsafe impl Send for Document {}
unsafe impl Sync for Document {}

#[derive(Clone)]
struct DocumentHandle(Weak<DocumentInner>);

impl DocumentHandle {
    fn authors(&self) -> Option<Authors> {
        self.0.upgrade().map(|arc| Document(arc).authors().clone())
    }
}

impl SubscribableTopic for DocumentHandle {
    fn bytes_received(&self, author: p2panda_core::PublicKey, data: Vec<u8>) {
        if let Some(document) = self.0.upgrade() {
            let doc = Document(document);
            doc.on_remote_message(data);
            doc.authors().add(PublicKey(author));
        }
    }

    fn author_joined(&self, author: p2panda_core::PublicKey) {
        if let Some(document) = self.0.upgrade() {
            let doc = Document(document);
            let author = doc.authors().add(PublicKey(author));
            author.set_online(true);
            doc.broadcast_ephemeral();
        }
    }

    fn author_left(&self, author: p2panda_core::PublicKey) {
        if let Some(document) = self.0.upgrade() {
            let doc = Document(document);
            let author = doc.authors().add(PublicKey(author));
            author.set_online(false);
        }
    }

    fn ephemeral_bytes_received(&self, author: p2panda_core::PublicKey, data: Vec<u8>) {
        let Some(document) = self.0.upgrade() else {
            return;
        };

        if let Ok(EphemeralData::Cursor {
            insert_cursor,
            selection_bound,
            timestamp,
        }) = decode_cbor(&data[..])
        {
            if let Some(authors) = self.authors() {
                if let Some(author) = authors.author(&PublicKey(author)) {
                    if !author.is_new_cursor_position(timestamp) {
                        return;
                    }
                }
            }

            *document.insert_cursor.lock().unwrap() = insert_cursor;
            *document.selection_bound.lock().unwrap() = selection_bound;
        }
    }
}
