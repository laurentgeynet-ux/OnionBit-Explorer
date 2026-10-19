// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `JsonFileStore` — persistance du profil en fichiers JSON
//! dans un répertoire déplaçable (`EXPLORER_PROFILE`) :
//! `history.json` (visites, bornées) et `bookmarks.json`
//! (signets). Implémente le port `ProfileStore` de
//! `explorer-core` — l'I/O fichier vit ici, jamais dans le
//! cœur.
//!
//! Format volontairement simple : un tableau JSON
//! `[{"url","title"}]` réécrit en entier à chaque mutation
//! (volumes MVP ; l'historique est plafonné).

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use explorer_core::{Bookmark, ProfileStore, Visit};

/// Borne du journal de visites (règle 13 : constante de
/// config, pas de magie dans la boucle).
const HISTORY_CAP: usize = 5000;

/// Store fichiers : `dir/history.json` + `dir/bookmarks.json`.
/// Mutex interne — écritures sérialisées (le `Session`
/// single-writer rend les courses improbables, mais le port
/// est `Sync`).
pub struct JsonFileStore {
    dir: PathBuf,
    /// Sérialise les écritures de fichiers.
    io: Mutex<()>,
    /// Journal en mémoire — miroir de `history.json`.
    history: Mutex<Vec<Visit>>,
}

impl JsonFileStore {
    /// Ouvre (ou crée) le répertoire de profil `dir` et
    /// recharge l'historique existant — tolérant : un fichier
    /// illisible repart d'un journal vide (jamais de panic).
    #[must_use]
    pub fn open(dir: &Path) -> Self {
        let _ = fs::create_dir_all(dir);
        let history = read_json(&dir.join("history.json"))
            .and_then(|v| serde_json::from_value::<Vec<RawEntry>>(v).ok())
            .map(|v| {
                v.into_iter()
                    .map(|e| Visit {
                        url: e.url,
                        title: e.title,
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self {
            dir: dir.to_path_buf(),
            io: Mutex::new(()),
            history: Mutex::new(history),
        }
    }

    /// Répertoire de profil par défaut selon la plateforme
    /// (`EXPLORER_PROFILE` prioritaire — déplaçable).
    #[must_use]
    pub fn default_dir() -> PathBuf {
        if let Some(d) = std::env::var_os("EXPLORER_PROFILE") {
            return PathBuf::from(d);
        }
        #[cfg(windows)]
        {
            std::env::var_os("APPDATA").map_or_else(
                || PathBuf::from(r".\explorer-profile"),
                |d| PathBuf::from(d).join("explorer"),
            )
        }
        #[cfg(target_os = "macos")]
        {
            std::env::var_os("HOME").map_or_else(
                || PathBuf::from("./explorer-profile"),
                |d| PathBuf::from(d).join("Library/Application Support/explorer"),
            )
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            std::env::var_os("XDG_DATA_HOME")
                .or_else(|| {
                    std::env::var_os("HOME").map(|h| {
                        let mut p = PathBuf::from(h);
                        p.push(".local/share");
                        p.into()
                    })
                })
                .map_or_else(
                    || PathBuf::from("./explorer-profile"),
                    |d| PathBuf::from(d).join("explorer"),
                )
        }
    }

    /// Réécrit `name` avec le JSON sérialisé de `entries` —
    /// écriture atomique best-effort (tmp + rename).
    fn write_entries(&self, name: &str, entries: &[RawEntry]) {
        let _guard = self.io.lock().ok();
        let Some(s) = serde_json::to_string(entries).ok() else {
            return;
        };
        let dst = self.dir.join(name);
        let tmp = self.dir.join(format!("{name}.tmp"));
        if fs::write(&tmp, s).is_ok() {
            let _ = fs::rename(&tmp, &dst);
        }
    }
}

impl ProfileStore for JsonFileStore {
    fn record_visit(&self, url: &str, title: Option<&str>) {
        if let Ok(mut h) = self.history.lock() {
            h.push(Visit {
                url: url.to_string(),
                title: title.map(str::to_string),
            });
            // Borne : on tronque par le début (les plus
            // anciennes sortent).
            if h.len() > HISTORY_CAP {
                let over = h.len() - HISTORY_CAP;
                h.drain(..over);
            }
            let entries: Vec<RawEntry> = h.iter().map(RawEntry::from).collect();
            drop(h);
            self.write_entries("history.json", &entries);
        }
    }

    fn load_bookmarks(&self) -> Vec<Bookmark> {
        read_json(&self.dir.join("bookmarks.json"))
            .and_then(|v| serde_json::from_value::<Vec<RawEntry>>(v).ok())
            .map(|v| {
                v.into_iter()
                    .map(|e| Bookmark {
                        url: e.url,
                        title: e.title,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn save_bookmarks(&self, b: &[Bookmark]) {
        let entries: Vec<RawEntry> = b.iter().map(RawEntry::from).collect();
        self.write_entries("bookmarks.json", &entries);
    }

    fn history(&self) -> Vec<Visit> {
        self.history
            .lock()
            .map_or_else(|_| Vec::new(), |h| h.clone())
    }
}

/// Forme sérialisée commune `{"url", "title"}` — visites et
/// signets partagent le même schéma.
#[derive(serde::Serialize, serde::Deserialize)]
struct RawEntry {
    url: String,
    #[serde(default)]
    title: Option<String>,
}

impl From<&Visit> for RawEntry {
    fn from(v: &Visit) -> Self {
        Self {
            url: v.url.clone(),
            title: v.title.clone(),
        }
    }
}

impl From<&Bookmark> for RawEntry {
    fn from(b: &Bookmark) -> Self {
        Self {
            url: b.url.clone(),
            title: b.title.clone(),
        }
    }
}

/// Lit un JSON depuis `p` — `None` si absent/illisible.
fn read_json(p: &Path) -> Option<serde_json::Value> {
    let bytes = fs::read(p).ok()?;
    serde_json::from_slice(&bytes).ok()
}
