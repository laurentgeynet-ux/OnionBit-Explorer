// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Persistance du profil (étape 56) — `JsonFileStore` et
//! `DiskCache` sur répertoires temporaires : cookies qui
//! survivent à une « réouverture », fichiers cassés qui
//! dégradent en vide, cache disque borné avec éviction FIFO.

use std::fs;
use std::time::SystemTime;

use explorer_core::{ProfileStore, StoredCookie};
use explorer_daemon::cache::DiskCache;
use explorer_daemon::store::JsonFileStore;
use explorer_net::cache::{CacheConfig, CacheEntry, CacheStore};

/// Répertoire temporaire unique par test (pas de dev-dep
/// `tempfile` — `std::env::temp_dir` + nom dérivé du test).
fn tmp_dir(name: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("explorer-e56-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    d
}

/// Cookie de test persistable.
fn sc(name: &str, value: &str) -> StoredCookie {
    StoredCookie {
        name: name.to_string(),
        value: value.to_string(),
        domain: "h.test".to_string(),
        host_only: true,
        path: "/".to_string(),
        expires_unix: Some(4_000_000_000),
        secure: false,
        http_only: false,
        same_site: "lax".to_string(),
        partition: None,
    }
}

/// Entrée de cache de test : corps `n` octets, pas de
/// fraîcheur (revalidation).
fn entry(n: usize) -> CacheEntry {
    CacheEntry {
        status: 200,
        reason: "OK".to_string(),
        headers: vec![("content-type".to_string(), "text/plain".to_string())],
        body: vec![b'x'; n],
        stored_at: SystemTime::now(),
        fresh_until: None,
        etag: Some("\"e\"".to_string()),
        last_modified: None,
    }
}

#[test]
fn cookies_roundtrip_through_store() {
    let dir = tmp_dir("cookies");
    let store = JsonFileStore::open(&dir);
    store.save_cookies(&[sc("a", "1"), sc("b", "2")]);

    // « Réouverture » : le fichier relu rend le même jar.
    let store2 = JsonFileStore::open(&dir);
    let got = store2.load_cookies();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].name, "a");
    assert_eq!(got[1].value, "2");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn corrupted_files_degrade_to_empty() {
    let dir = tmp_dir("corrupt");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("cookies.json"), "{{{{pas du json").unwrap();
    fs::write(dir.join("history.json"), "[").unwrap();
    fs::write(dir.join("storage-https_h.test.json"), "nope").unwrap();

    let store = JsonFileStore::open(&dir);
    assert!(store.load_cookies().is_empty());
    assert!(store.history().is_empty());
    assert!(store.load_storage("https://h.test").is_empty());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn disk_cache_put_get_and_reopen() {
    let dir = tmp_dir("cache");
    let cache_dir = dir.join("cache");
    let c = DiskCache::open(&cache_dir, CacheConfig::default());
    c.cache_put("http://h.test/a", entry(64));

    let e = c.cache_get("http://h.test/a").unwrap();
    assert_eq!(e.status, 200);
    assert_eq!(e.body.len(), 64);
    assert_eq!(e.etag.as_deref(), Some("\"e\""));
    assert!(c.cache_get("http://h.test/miss").is_none());

    // Réouverture : l'index se reconstruit depuis le disque.
    let c2 = DiskCache::open(&cache_dir, CacheConfig::default());
    assert_eq!(c2.cache_get("http://h.test/a").unwrap().body.len(), 64);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn disk_cache_evicts_fifo_over_budget() {
    let dir = tmp_dir("evict");
    let cfg = CacheConfig {
        max_entry_bytes: 1024,
        max_total_bytes: 300,
    };
    let c = DiskCache::open(&dir.join("cache"), cfg);
    // Trois entrées de 200 octets → 600 > 300 : les plus
    // anciennes sortent jusqu'au budget.
    for i in 0..3 {
        c.cache_put(&format!("http://h.test/{i}"), entry(200));
    }
    assert!(
        c.cache_get("http://h.test/0").is_none(),
        "plus ancien évincé"
    );
    assert!(c.cache_get("http://h.test/1").is_none(), "second évincé");
    assert!(c.cache_get("http://h.test/2").is_some(), "récent conservé");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn disk_cache_ignores_oversized_and_broken_meta() {
    let dir = tmp_dir("broken");
    let cfg = CacheConfig {
        max_entry_bytes: 100,
        max_total_bytes: CacheConfig::default().max_total_bytes,
    };
    let cache_dir = dir.join("cache");
    let c = DiskCache::open(&cache_dir, cfg);
    // Corps > borne entrée → jamais écrit.
    c.cache_put("http://h.test/big", entry(500));
    assert!(c.cache_get("http://h.test/big").is_none());

    // Méta cassée sur disque → entrée ignorée, pas de panic.
    c.cache_put("http://h.test/ok", entry(10));
    fs::write(cache_dir.join("cassé.json"), "{").unwrap();
    let c2 = DiskCache::open(&cache_dir, CacheConfig::default());
    assert!(c2.cache_get("http://h.test/ok").is_some());
    let _ = fs::remove_dir_all(&dir);
}
