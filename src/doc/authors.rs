use std::sync::{Arc, RwLock};

use chrono::{DateTime, Utc};
use indexmap::IndexMap;

use super::author::Author;
use super::identity::PublicKey;

#[derive(Default, Clone)]
pub struct Authors {
    list: Arc<RwLock<IndexMap<PublicKey, Author>>>,
}

impl Authors {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load(&self, authors: Vec<Author>) {
        let mut list = self.list.write().unwrap();
        assert!(list.len() <= 1);

        for author in authors {
            let public_key = author.public_key();
            list.entry(public_key).or_insert(author);
        }
    }

    pub fn add_this_device(&self, author_key: PublicKey, last_seen: Option<DateTime<Utc>>) {
        let mut list = self.list.write().unwrap();
        assert!(list.is_empty());

        let author = Author::for_this_device(&author_key, last_seen.as_ref());
        list.insert(author_key, author);
    }

    pub fn add(&self, author_key: PublicKey) -> Author {
        let mut list = self.list.write().unwrap();
        list.entry(author_key)
            .or_insert_with_key(Author::new)
            .clone()
    }
}
