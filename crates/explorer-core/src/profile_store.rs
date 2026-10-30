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

use crate::{Bookmark, DownloadRecord, ProfileStore, StoredCookie, Visit};

/// Borne du journal de visites (règle 13 : constante de
/// config, pas de magie dans la boucle).
const HISTORY_CAP: usize = 5000;

/// Borne du journal de téléchargements (PR-1).
const DOWNLOADS_CAP: usize = 2000;

/// Bornes autofill (PR-6) : valeurs par nom / noms distincts.
const AUTOFILL_VALUES_CAP: usize = 10;
const AUTOFILL_NAMES_CAP: usize = 2000;

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
    /// Journal des téléchargements — miroir de
    /// `downloads.json` (PR-1).
    downloads: Mutex<Vec<DownloadRecord>>,
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
        let downloads = read_json(&dir.join("downloads.json"))
            .and_then(|v| serde_json::from_value::<Vec<RawDownload>>(v).ok())
            .map(|v| v.into_iter().map(RawDownload::into_record).collect())
            .unwrap_or_default();
        Self {
            dir: dir.to_path_buf(),
            io: Mutex::new(()),
            history: Mutex::new(history),
            downloads: Mutex::new(downloads),
        }
    }

    /// Répertoire de profil par défaut selon la plateforme
    /// (`EXPLORER_PROFILE` prioritaire — déplaçable ; puis bundle
    /// portable détecté par le marqueur
    /// [`crate::paths::PORTABLE_MARKER`] → `<racine>/state`).
    #[must_use]
    pub fn default_dir() -> PathBuf {
        if let Some(d) = std::env::var_os("EXPLORER_PROFILE") {
            return PathBuf::from(d);
        }
        if let Some(d) = crate::paths::portable_state_dir() {
            return d;
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

    /// Journal des téléchargements (PR-1) : `downloads.json` —
    /// `path` réécrit **relatif au profil** quand le fichier y
    /// vit (invariant portable : rien d'absolu persisté — la
    /// lettre de clé USB change).
    fn record_download(&self, d: &DownloadRecord) {
        if let Ok(mut v) = self.downloads.lock() {
            let mut rec = d.clone();
            rec.path = match std::path::Path::new(&d.path).strip_prefix(&self.dir) {
                Ok(rel) => rel.to_string_lossy().replace('\\', "/"),
                Err(_) => d.path.clone(),
            };
            v.push(rec);
            if v.len() > DOWNLOADS_CAP {
                let over = v.len() - DOWNLOADS_CAP;
                v.drain(..over);
            }
            let entries: Vec<RawDownload> = v.iter().map(RawDownload::from).collect();
            drop(v);
            let _guard = self.io.lock().ok();
            let Some(s) = serde_json::to_string(&entries).ok() else {
                return;
            };
            let dst = self.dir.join("downloads.json");
            let tmp = self.dir.join("downloads.json.tmp");
            if fs::write(&tmp, s).is_ok() {
                let _ = fs::rename(&tmp, &dst);
            }
        }
    }

    fn downloads(&self) -> Vec<DownloadRecord> {
        self.downloads
            .lock()
            .map_or_else(|_| Vec::new(), |v| v.clone())
    }

    /// Autofill (PR-6) : `autofill.json` =
    /// `{nom → [valeurs]}` — plus récente en tête, bornée à
    /// [`AUTOFILL_VALUES_CAP`] par nom et [`AUTOFILL_NAMES_CAP`]
    /// noms (éviction arbitraire quand la map déborde — dev
    /// store, ADR-0030). **En clair** : à sceller par
    /// `ProfileVault` avant release prod.
    fn record_autofill(&self, name: &str, value: &str) {
        if name.is_empty() || value.trim().is_empty() {
            return;
        }
        let _guard = self.io.lock().ok();
        let mut map: std::collections::HashMap<String, Vec<String>> =
            read_json(&self.dir.join("autofill.json"))
                .and_then(|v| serde_json::from_value(v).ok())
                .unwrap_or_default();
        let vals = map.entry(name.to_string()).or_default();
        // Déduplication par valeur : la plus récente en tête.
        vals.retain(|v| v != value);
        vals.insert(0, value.to_string());
        vals.truncate(AUTOFILL_VALUES_CAP);
        if map.len() > AUTOFILL_NAMES_CAP {
            let extra: Vec<String> = map
                .keys()
                .take(map.len() - AUTOFILL_NAMES_CAP)
                .cloned()
                .collect();
            for k in extra {
                map.remove(&k);
            }
        }
        let Some(s) = serde_json::to_string(&map).ok() else {
            return;
        };
        let dst = self.dir.join("autofill.json");
        let tmp = self.dir.join("autofill.json.tmp");
        if fs::write(&tmp, s).is_ok() {
            let _ = fs::rename(&tmp, &dst);
        }
    }

    fn autofill(&self, name: &str) -> Vec<String> {
        read_json(&self.dir.join("autofill.json"))
            .and_then(|v| {
                serde_json::from_value::<std::collections::HashMap<String, Vec<String>>>(v).ok()
            })
            .and_then(|m| m.get(name).cloned())
            .unwrap_or_default()
    }

    /// `idb-<slug>.json` (PR-10) — snapshot `IndexedDB` brut de
    /// l'origine ; contenu opaque côté store (le moteur parse).
    fn load_idb(&self, origin: &str) -> String {
        fs::read_to_string(self.dir.join(idb_name(origin))).unwrap_or_default()
    }

    fn save_idb(&self, origin: &str, json: &str) {
        let _guard = self.io.lock().ok();
        let dst = self.dir.join(idb_name(origin));
        let tmp = self.dir.join(format!("{}.tmp", idb_name(origin)));
        if fs::write(&tmp, json).is_ok() {
            let _ = fs::rename(&tmp, &dst);
        }
    }
}

/// Forme sérialisée d'une entrée `downloads.json` (PR-1).
#[derive(serde::Serialize, serde::Deserialize)]
struct RawDownload {
    url: String,
    #[serde(default)]
    filename: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    finished_unix: u64,
}

impl From<&DownloadRecord> for RawDownload {
    fn from(d: &DownloadRecord) -> Self {
        Self {
            url: d.url.clone(),
            filename: d.filename.clone(),
            path: d.path.clone(),
            size: d.size,
            finished_unix: d.finished_unix,
        }
    }
}

impl RawDownload {
    fn into_record(self) -> DownloadRecord {
        DownloadRecord {
            url: self.url,
            filename: self.filename,
            path: self.path,
            size: self.size,
            finished_unix: self.finished_unix,
        }
    }
}

/// Store HSTS persisté (`hsts.json`, PR-3) — `host → {exp,
/// subs}` ; implémente le port
/// [`explorer_net::hsts::HstsStore`] injecté dans la pile
/// transport. `is_secure` couvre l'hôte exact et les domaines
/// parents `includeSubDomains` non expirés ; les entrées
/// périmées sont purgées à la lecture et ignorées partout.
pub struct JsonHstsStore {
    dir: PathBuf,
    /// `host → (expires_unix, include_subdomains)`.
    map: Mutex<std::collections::HashMap<String, RawHsts>>,
    /// Sérialise les écritures.
    io: Mutex<()>,
}

impl JsonHstsStore {
    /// Ouvre (ou crée) `<dir>/hsts.json` — entrées expirées
    /// élaguées à la lecture, fichier cassé → map vide.
    #[must_use]
    pub fn open(dir: &Path) -> Self {
        let _ = fs::create_dir_all(dir);
        let now = unix_now();
        let map = read_json(&dir.join("hsts.json"))
            .and_then(|v| {
                serde_json::from_value::<std::collections::HashMap<String, RawHsts>>(v).ok()
            })
            .map(|m| m.into_iter().filter(|(_, e)| e.exp > now).collect())
            .unwrap_or_default();
        Self {
            dir: dir.to_path_buf(),
            map: Mutex::new(map),
            io: Mutex::new(()),
        }
    }

    /// `true` si `host` (ou un parent `subs`) a une entrée
    /// active.
    fn covered(map: &std::collections::HashMap<String, RawHsts>, host: &str) -> bool {
        let now = unix_now();
        let mut h = host;
        loop {
            if let Some(e) = map.get(h) {
                if e.exp > now && (h == host || e.subs) {
                    return true;
                }
            }
            match h.split_once('.') {
                Some((_, parent)) => h = parent,
                None => return false,
            }
        }
    }
}

impl explorer_net::hsts::HstsStore for JsonHstsStore {
    fn is_secure(&self, host: &str) -> bool {
        self.map.lock().is_ok_and(|m| Self::covered(&m, host))
    }

    /// RFC 6797 §8.1 : `max-age=0` retire la politique ; sinon
    /// `host → exp` réécrit (`hsts.json`, atomique).
    fn record(&self, host: &str, max_age: u64, include_subdomains: bool) {
        if let Ok(mut m) = self.map.lock() {
            if max_age == 0 {
                m.remove(host);
            } else {
                m.insert(
                    host.to_string(),
                    RawHsts {
                        exp: unix_now() + max_age,
                        subs: include_subdomains,
                    },
                );
            }
            let snapshot: std::collections::HashMap<String, RawHsts> = m.clone();
            drop(m);
            let _guard = self.io.lock().ok();
            let Some(s) = serde_json::to_string(&snapshot).ok() else {
                return;
            };
            let dst = self.dir.join("hsts.json");
            let tmp = self.dir.join("hsts.json.tmp");
            if fs::write(&tmp, s).is_ok() {
                let _ = fs::rename(&tmp, &dst);
            }
        }
    }
}

/// Entrée `hsts.json` sérialisée.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct RawHsts {
    /// Expiration — secondes unix.
    exp: u64,
    /// `includeSubDomains`.
    #[serde(default)]
    subs: bool,
}

/// Store DNS persisté (`dns_cache.json`, PR-9) —
/// `host → {ips, exp}` ; implémente le port
/// [`explorer_net::resolver::DnsStore`] injecté dans
/// `CachingResolver`. Borné : entrées périmées élaguées à la
/// lecture et à chaque hit, map plafonnée — les plus anciennes
/// expirations sortent d'abord.
pub struct JsonDnsStore {
    dir: PathBuf,
    /// `host → (ips, expires_unix)`.
    map: Mutex<std::collections::HashMap<String, RawDns>>,
    /// Sérialise les écritures.
    io: Mutex<()>,
}

/// Borne du cache DNS persisté (PR-9).
const DNS_CACHE_CAP: usize = 4000;

impl JsonDnsStore {
    /// Ouvre (ou crée) `<dir>/dns_cache.json` — entrées
    /// expirées élaguées à la lecture.
    #[must_use]
    pub fn open(dir: &Path) -> Self {
        let _ = fs::create_dir_all(dir);
        let now = unix_now();
        let map = read_json(&dir.join("dns_cache.json"))
            .and_then(|v| {
                serde_json::from_value::<std::collections::HashMap<String, RawDns>>(v).ok()
            })
            .map(|m| m.into_iter().filter(|(_, e)| e.exp > now).collect())
            .unwrap_or_default();
        Self {
            dir: dir.to_path_buf(),
            map: Mutex::new(map),
            io: Mutex::new(()),
        }
    }
}

impl explorer_net::resolver::DnsStore for JsonDnsStore {
    fn lookup(&self, host: &str) -> Option<Vec<std::net::IpAddr>> {
        self.map.lock().ok().and_then(|mut m| {
            let e = m.get(host)?;
            if e.exp <= unix_now() {
                m.remove(host);
                return None;
            }
            Some(
                e.ips
                    .iter()
                    .filter_map(|s| s.parse::<std::net::IpAddr>().ok())
                    .collect(),
            )
        })
    }

    fn record(&self, host: &str, ips: &[std::net::IpAddr], ttl: std::time::Duration) {
        if let Ok(mut m) = self.map.lock() {
            m.insert(
                host.to_string(),
                RawDns {
                    ips: ips.iter().map(ToString::to_string).collect(),
                    exp: unix_now() + ttl.as_secs(),
                },
            );
            // Borne : hors capacité, les expirations les plus
            // proches (les plus périmées) sortent d'abord.
            if m.len() > DNS_CACHE_CAP {
                let mut by_exp: Vec<(u64, String)> =
                    m.iter().map(|(h, e)| (e.exp, h.clone())).collect();
                by_exp.sort_unstable();
                for (_, h) in by_exp.into_iter().take(m.len() - DNS_CACHE_CAP) {
                    m.remove(&h);
                }
            }
            let snapshot = m.clone();
            drop(m);
            let _guard = self.io.lock().ok();
            let Some(s) = serde_json::to_string(&snapshot).ok() else {
                return;
            };
            let dst = self.dir.join("dns_cache.json");
            let tmp = self.dir.join("dns_cache.json.tmp");
            if fs::write(&tmp, s).is_ok() {
                let _ = fs::rename(&tmp, &dst);
            }
        }
    }
}

/// Entrée `dns_cache.json` sérialisée (PR-9).
#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct RawDns {
    /// Adresses IP textuelles.
    ips: Vec<String>,
    /// Expiration — secondes unix.
    exp: u64,
}

/// Secondes unix courantes.
fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
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

/// `idb-<slug>.json` — même slug que `storage_name` (PR-10).
fn idb_name(origin: &str) -> String {
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
    format!("idb-{slug}.json")
}
