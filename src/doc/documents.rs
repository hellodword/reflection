use std::sync::{Arc, RwLock};

use chrono::{DateTime, Utc};
use indexmap::IndexMap;

use super::author::Author;
use super::document::{Document, DocumentId};
use super::identity::PublicKey;
use super::service::Service;

#[derive(Default, Clone)]
pub struct Documents {
    list: Arc<RwLock<IndexMap<DocumentId, Document>>>,
}

impl Documents {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn load(&self, service: &Service) -> Result<(), super::service::StartupError> {
        let public_key = service.private_key().public_key();

        let documents = service.node().topics::<DocumentId>().await?;

        let mut list = self.list.write().unwrap();
        assert!(list.is_empty());

        for document in documents {
            let last_accessed = document.last_accessed;

            let authors: Vec<Author> = document
                .authors
                .iter()
                .map(|author| {
                    let author_public_key = PublicKey(author.public_key);
                    let last_seen = author.last_seen.and_then(|last_seen| {
                        DateTime::<Utc>::from_timestamp(last_seen.timestamp(), 0)
                    });

                    if author_public_key == public_key {
                        Author::for_this_device(&author_public_key, last_seen.as_ref())
                    } else {
                        Author::with_state(&author_public_key, last_seen.as_ref())
                    }
                })
                .collect();

            let obj = Document::new(service, &document.id);
            obj.load_authors(authors);
            obj.set_last_accessed(last_accessed);

            list.insert(document.id, obj);
        }

        Ok(())
    }

    pub fn add(&self, document: Document) {
        let mut list = self.list.write().unwrap();
        let document_id = document.id();

        if list.contains_key(&document_id) {
            return;
        }

        list.insert(document_id, document);
    }

    pub fn document(&self, document_id: &DocumentId) -> Option<Document> {
        let list = self.list.read().unwrap();

        list.get(document_id).cloned()
    }

    pub fn iter(&self) -> Vec<Document> {
        self.list.read().unwrap().values().cloned().collect()
    }
}
