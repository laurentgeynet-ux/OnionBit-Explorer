// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `JsonFileStore` — persistance du profil en fichiers JSON
//! dans un répertoire déplaçable (`EXPLORER_PROFILE`) :
//! `history.json` (visites, bornées), `bookmarks.json`
//! (signets), `storage-<slug>.json` (localStorage par
//! origine) et `cookies.json` (jar à cookies, étape 56).
//! Implémente le port [`ProfileStore`].
//!
//! Frontière (étape 61) : le type vit dans `explorer-core`
//! — et non chez un hôte — parce que **deux** adaptateurs
//! l'injectent (`explorer-daemon` via `set_store`,
//! `explorer-ffi` depuis sa lecture de `EXPLORER_PROFILE`),
//! et aucun crate partagé plus bas ne peut voir les DTO
//! sans dépendance montante. Le cœur reste néanmoins sans
//! I/O en propre : `Session` ne touche jamais le système
//! de fichiers — l'impl est toujours *injectée* ; et les
//! DTO publiques restent neutres (les formes filaires
//! `Raw*` serde restent privées au module).
//!
//! Format volontairement simple : un tableau JSON
//! `[{"url","title"}]` réécrit en entier à chaque mutation
//! (volumes MVP ; l'historique est plafonné).

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use crate::{Bookmark, ProfileStore, StoredCookie, Visit};

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

    /// `localStorage` par origine (étape 51) : fichier
    /// `storage-<slug>.json` = objet `{clé: valeur}` —
    /// illisible/cassé → origine vide (dégradation).
    fn load_storage(&self, origin: &str) -> BTreeMap<String, String> {
        read_json(&self.dir.join(storage_name(origin)))
            .and_then(|v| serde_json::from_value::<BTreeMap<String, String>>(v).ok())
            .unwrap_or_default()
    }

    fn save_storage(&self, origin: &str, map: &BTreeMap<String, String>) {
        let _guard = self.io.lock().ok();
        let Some(s) = serde_json::to_string(map).ok() else {
            return;
        };
        let dst = self.dir.join(storage_name(origin));
        let tmp = self.dir.join(format!("{}.tmp", storage_name(origin)));
        if fs::write(&tmp, s).is_ok() {
            let _ = fs::rename(&tmp, &dst);
        }
    }

    /// Jar à cookies (étape 56) : `cookies.json` = tableau
    /// de `RawCookie` — fichier cassé → jar vide. Le core
    /// appelle `save_cookies` à chaque point de synchro où
    /// le jar a pu muter ; la liste est bornée en amont par
    /// `CookieConfig`.
    fn load_cookies(&self) -> Vec<StoredCookie> {
        read_json(&self.dir.join("cookies.json"))
            .and_then(|v| serde_json::from_value::<Vec<RawCookie>>(v).ok())
            .map(|v| v.into_iter().map(RawCookie::into_stored).collect())
            .unwrap_or_default()
    }

    fn save_cookies(&self, cookies: &[StoredCookie]) {
        let entries: Vec<RawCookie> = cookies.iter().map(RawCookie::from).collect();
        let _guard = self.io.lock().ok();
        let Some(s) = serde_json::to_string(&entries).ok() else {
            return;
        };
        let dst = self.dir.join("cookies.json");
        let tmp = self.dir.join("cookies.json.tmp");
        if fs::write(&tmp, s).is_ok() {
            let _ = fs::rename(&tmp, &dst);
        }
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

/// Cookie sérialisé (`cookies.json`) — miroir serde de
/// `StoredCookie` : le type du core reste neutre (pas de
/// serde dans `explorer-core`), la forme filaire vit ici.
#[derive(serde::Serialize, serde::Deserialize)]
struct RawCookie {
    name: String,
    value: String,
    domain: String,
    #[serde(default)]
    host_only: bool,
    path: String,
    #[serde(default)]
    expires_unix: Option<u64>,
    #[serde(default)]
    secure: bool,
    #[serde(default)]
    http_only: bool,
    #[serde(default)]
    same_site: String,
    /// Clé de partition CHIPS (étape 127) — `default` : les
    /// profils écrits avant l'étape relisent `None`
    /// (cookie non partitionné), aucune migration.
    #[serde(default)]
    partition: Option<String>,
}

impl RawCookie {
    /// Forme filaire → type du core (pas de validation ici :
    /// `CookieJar::insert_stored` revérifie l'expiration et
    /// les invariants au réimport).
    fn into_stored(self) -> StoredCookie {
        StoredCookie {
            name: self.name,
            value: self.value,
            domain: self.domain,
            host_only: self.host_only,
            path: self.path,
            expires_unix: self.expires_unix,
            secure: self.secure,
            http_only: self.http_only,
            same_site: self.same_site,
            partition: self.partition,
        }
    }
}

impl From<&StoredCookie> for RawCookie {
    fn from(c: &StoredCookie) -> Self {
        Self {
            name: c.name.clone(),
            value: c.value.clone(),
            domain: c.domain.clone(),
            host_only: c.host_only,
            path: c.path.clone(),
            expires_unix: c.expires_unix,
            secure: c.secure,
            http_only: c.http_only,
            same_site: c.same_site.clone(),
            partition: c.partition.clone(),
        }
    }
}

/// Lit un JSON depuis `p` — `None` si absent/illisible.
fn read_json(p: &Path) -> Option<serde_json::Value> {
    let bytes = fs::read(p).ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// Nom de fichier storage d'une origine — caractères sûrs
/// seulement (`https://a.com` → `storage-https_a_com.json`),
/// borné à 64 caractères (noms de fichiers raisonnables).
fn storage_name(origin: &str) -> String {
    let slug: String = origin
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    format!("storage-{slug}.json")
}
