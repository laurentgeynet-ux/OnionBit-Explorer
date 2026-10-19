// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `DiskCache` — cache HTTP persisté et borné (étape 56) :
//! implémente le port `CacheStore` de `explorer-net`. Chaque
//! entrée = `cache/<fnv64>.json` (méta : statut, en-têtes,
//! validateurs, fraîcheur) + `cache/<fnv64>.bin` (corps) —
//! la politique de fraîcheur/revalidation reste dans
//! `CachingTransport` ; ici, seule la durabilité.
//!
//! Index mémoire reconstruit à l'ouverture (scan du
//! répertoire) ; éviction FIFO par `stored_at` quand le total
//! dépasse `CacheConfig::max_total_bytes`. Tout I/O est
//! best-effort : fichier cassé = entrée ignorée, écriture
//! ratée = fetch normal — jamais de panic, jamais de refus.

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use explorer_net::cache::{CacheConfig, CacheEntry, CacheStore};

/// Entrée mémoire de l'index : fichier + taille + rang
/// d'insertion (pour l'éviction FIFO).
struct Slot {
    /// Fichier de méta `<hash>.json` (le corps vit à côté,
    /// `<hash>.bin`).
    meta: PathBuf,
    /// Taille du corps (octets — lue au scan, pas relue).
    size: usize,
}

/// Cache disque : `dir` = `<profil>/cache`. Mutex interne —
/// le transport est `Sync`, les accès sont courts (jamais
/// d'I/O réseau sous le verrou, mais la lecture fichier
/// d'une entrée se fait sous verrou : le disque profil est
/// local, la contention est négligeable au MVP).
pub struct DiskCache {
    dir: PathBuf,
    cfg: CacheConfig,
    /// Index `clé → slot` + ordre FIFO des clés.
    index: Mutex<(HashMap<String, Slot>, VecDeque<String>)>,
}

impl DiskCache {
    /// Ouvre (ou crée) `dir` et reconstruit l'index —
    /// tolérant : méta illisible → entrée ignorée ; corps
    /// manquant → slot sans corps, servi seulement si
    /// jamais demandé.
    #[must_use]
    pub fn open(dir: &Path, cfg: CacheConfig) -> Self {
        let _ = fs::create_dir_all(dir);
        let mut index = HashMap::new();
        let mut order = VecDeque::new();
        if let Ok(rd) = fs::read_dir(dir) {
            for ent in rd.flatten() {
                let p = ent.path();
                if p.extension().and_then(|e| e.to_str()) != Some("json") {
                    continue;
                }
                let Some(meta) = read_meta(&p) else { continue };
                let stored = meta.stored_at();
                let hash = meta.hash.clone();
                let Some(key) = meta.key else { continue };
                let size = fs::metadata(bin_path(dir, &hash))
                    .map_or(0, |m| usize::try_from(m.len()).unwrap_or(usize::MAX));
                index.insert(key.clone(), Slot { meta: p, size });
                order.push_back((stored, key));
            }
        }
        // Ordre FIFO par date de stockage.
        let mut ord: Vec<(SystemTime, String)> = order.into_iter().collect();
        ord.sort();
        let order = ord.into_iter().map(|(_, k)| k).collect();
        Self {
            dir: dir.to_path_buf(),
            cfg,
            index: Mutex::new((index, order)),
        }
    }

    /// Écrit `entry` sous `key` (tmp + rename atomique
    /// best-effort) puis fait respecter `max_total_bytes`
    /// par éviction FIFO.
    fn put_entry(&self, key: &str, entry: &CacheEntry) {
        let hash = fnv64(key);
        let meta = RawMeta::of(key, &hash, entry);
        let Some(json) = serde_json::to_string(&meta).ok() else {
            return;
        };
        let mp = meta_path(&self.dir, &hash);
        let bp = bin_path(&self.dir, &hash);
        let tmp_m = mp.with_extension("json.tmp");
        let tmp_b = bp.with_extension("bin.tmp");
        if fs::write(&tmp_m, json).is_err() || fs::write(&tmp_b, &entry.body).is_err() {
            return;
        }
        let _ = fs::rename(&tmp_m, &mp);
        let _ = fs::rename(&tmp_b, &bp);
        let Ok(mut idx) = self.index.lock() else {
            return;
        };
        let (map, order) = &mut *idx;
        // Remplace : retirer l'ancien slot des compteurs.
        let mut total: usize = map.values().map(|s| s.size).sum();
        if let Some(old) = map.remove(key) {
            total = total.saturating_sub(old.size);
            order.retain(|k| k != key);
        }
        map.insert(
            key.to_string(),
            Slot {
                meta: mp,
                size: entry.body.len(),
            },
        );
        order.push_back(key.to_string());
        total += entry.body.len();
        // Éviction FIFO.
        while total > self.cfg.max_total_bytes {
            let Some(k) = order.pop_front() else { break };
            if let Some(slot) = map.remove(&k) {
                total = total.saturating_sub(slot.size);
                drop(fs::remove_file(&slot.meta));
                drop(fs::remove_file(slot.meta.with_extension("bin")));
            }
        }
    }
}

impl CacheStore for DiskCache {
    fn cache_get(&self, key: &str) -> Option<CacheEntry> {
        let hash = fnv64(key);
        let mp = meta_path(&self.dir, &hash);
        let meta = read_meta(&mp)?;
        let body = fs::read(bin_path(&self.dir, &hash)).ok()?;
        let stored_at = meta.stored_at();
        Some(CacheEntry {
            status: meta.status,
            reason: meta.reason,
            headers: meta.headers,
            body,
            stored_at,
            fresh_until: meta.fresh_unix.map(|s| UNIX_EPOCH + Duration::from_secs(s)),
            etag: meta.etag,
            last_modified: meta.last_modified,
        })
    }

    fn cache_put(&self, key: &str, entry: CacheEntry) {
        if entry.body.len() > self.cfg.max_entry_bytes {
            return;
        }
        self.put_entry(key, &entry);
    }
}

/// Méta sérialisée d'une entrée.
#[derive(serde::Serialize, serde::Deserialize)]
struct RawMeta {
    /// URL clé (vérifiée au scan — le nom de fichier n'est
    /// qu'un hachage).
    key: Option<String>,
    /// Hachage fnv de la clé — nomme le `.bin` voisin.
    hash: String,
    status: u16,
    reason: String,
    headers: Vec<(String, String)>,
    /// `stored_at` en secondes unix.
    stored_unix: u64,
    /// Fraîcheur en secondes unix (`None` = jamais frais —
    /// revalidation seule).
    fresh_unix: Option<u64>,
    #[serde(default)]
    etag: Option<String>,
    #[serde(default)]
    last_modified: Option<String>,
}

impl RawMeta {
    /// Méta in-memory de l'entrée → forme sérialisée.
    fn of(key: &str, hash: &str, e: &CacheEntry) -> Self {
        Self {
            key: Some(key.to_string()),
            hash: hash.to_string(),
            status: e.status,
            reason: e.reason.clone(),
            headers: e.headers.clone(),
            stored_unix: e
                .stored_at
                .duration_since(UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            fresh_unix: e
                .fresh_until
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs())),
            etag: e.etag.clone(),
            last_modified: e.last_modified.clone(),
        }
    }

    /// `stored_unix` → `SystemTime`.
    fn stored_at(&self) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(self.stored_unix)
    }
}

/// Lit et parse `<hash>.json` — `None` si absent/cassé.
fn read_meta(p: &Path) -> Option<RawMeta> {
    let bytes = fs::read(p).ok()?;
    serde_json::from_slice::<RawMeta>(&bytes).ok()
}

/// `<hash>.json` dans `dir`.
fn meta_path(dir: &Path, hash: &str) -> PathBuf {
    dir.join(format!("{hash}.json"))
}

/// `<hash>.bin` dans `dir`.
fn bin_path(dir: &Path, hash: &str) -> PathBuf {
    dir.join(format!("{hash}.bin"))
}

/// FNV-1a 64 bits de `s` — hachage déterministe de nom de
/// fichier (std `DefaultHasher` n'est pas stable entre
/// versions — interdit pour des fichiers persistants).
fn fnv64(s: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}
