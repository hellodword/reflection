use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::node::node::{ConnectionMode as NodeConnectionMode, Node, NodeError};
use crate::node::topic::TopicError;
use p2panda_core::Hash;
use thiserror::Error;
use tracing::error;

use super::document::{Document, DocumentId};
use super::documents::Documents;
use super::identity::PrivateKey;

#[derive(Error, Debug)]
pub enum StartupError {
    #[error(transparent)]
    Node(#[from] NodeError),
    #[error(transparent)]
    Topic(#[from] TopicError),
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Default)]
pub enum ConnectionMode {
    #[default]
    None,
    Network,
}

impl From<ConnectionMode> for NodeConnectionMode {
    fn from(value: ConnectionMode) -> Self {
        match value {
            ConnectionMode::None => NodeConnectionMode::None,
            ConnectionMode::Network => NodeConnectionMode::Network,
        }
    }
}

#[derive(Clone)]
pub struct Service {
    inner: Arc<ServiceInner>,
}

struct ServiceInner {
    node: Mutex<Option<Arc<Node>>>,
    private_key: PrivateKey,
    data_dir: Option<PathBuf>,
    documents: Documents,
    connection_mode: Mutex<ConnectionMode>,
}

impl Service {
    pub fn new(private_key: &PrivateKey, data_dir: Option<&Path>) -> Self {
        Service {
            inner: Arc::new(ServiceInner {
                node: Mutex::new(None),
                private_key: private_key.clone(),
                data_dir: data_dir.map(|p| p.to_path_buf()),
                documents: Documents::new(),
                connection_mode: Mutex::new(ConnectionMode::Network),
            }),
        }
    }

    pub fn private_key(&self) -> &PrivateKey {
        &self.inner.private_key
    }

    pub fn documents(&self) -> &Documents {
        &self.inner.documents
    }

    async fn update_node_connection_mode(&self) {
        let Some(node) = self.inner.node.lock().unwrap().clone() else {
            return;
        };
        let connection_mode: NodeConnectionMode =
            (*self.inner.connection_mode.lock().unwrap()).into();
        if let Err(err) = node.set_connection_mode(connection_mode).await {
            error!("Failed to set connection mode: {err}");
        }
    }

    pub async fn startup(&self) -> Result<(), StartupError> {
        let private_key = self.private_key().0.clone();
        let network_id = Hash::new(b"reflection");
        let path_opt = self.inner.data_dir.as_deref();
        let node = Node::new(private_key, network_id, path_opt, NodeConnectionMode::None).await?;

        *self.inner.node.lock().unwrap() = Some(Arc::new(node));

        self.update_node_connection_mode().await;
        self.documents().load(self).await?;

        Ok(())
    }

    pub async fn shutdown(&self) {
        for document in self.documents().iter() {
            document.unsubscribe().await;
        }

        if let Some(node) = self.inner.node.lock().unwrap().clone()
            && let Err(error) = node.shutdown().await
        {
            error!("Failed to shutdown service: {}", error);
        }
    }

    pub fn join_document(&self, document_id: &DocumentId) -> Document {
        let list = self.documents();
        if let Some(document) = list.document(document_id) {
            document
        } else {
            let document = Document::new(self, document_id);
            list.add(document.clone());
            document
        }
    }

    pub(crate) fn node(&self) -> Arc<Node> {
        self.inner
            .node
            .lock()
            .unwrap()
            .clone()
            .expect("Service to run")
    }
}
