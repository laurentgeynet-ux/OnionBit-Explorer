// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Store de profil : signets et historique persistés.
use super::*;

impl Session {
    /// Signets (miroir du store).
    #[must_use]
    pub fn bookmarks(&self) -> &[Bookmark] {
        &self.bookmarks
    }

    /// Ajoute un signet — dédupliqué par URL. Persiste via
    /// le store si branché.
    pub fn add_bookmark(&mut self, url: String, title: Option<String>) -> bool {
        if url.is_empty() || self.bookmarks.iter().any(|b| b.url == url) {
            return false;
        }
        self.bookmarks.push(Bookmark { url, title });
        if let Some(s) = &self.store {
            s.save_bookmarks(&self.bookmarks);
        }
        true
    }

    /// Retire un signet par URL — `true` s'il existait.
    pub fn remove_bookmark(&mut self, url: &str) -> bool {
        let n = self.bookmarks.len();
        self.bookmarks.retain(|b| b.url != url);
        if self.bookmarks.len() == n {
            false
        } else {
            if let Some(s) = &self.store {
                s.save_bookmarks(&self.bookmarks);
            }
            true
        }
    }

    /// Journal des visites du store — vide en mémoire pure.
    #[must_use]
    pub fn history(&self) -> Vec<Visit> {
        self.store.as_ref().map_or_else(Vec::new, |s| s.history())
    }
}
