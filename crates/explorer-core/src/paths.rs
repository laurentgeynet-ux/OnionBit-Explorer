// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Racine de bundle portable — marqueur `<racine>/Explorer.portable`.
//!
//! Layout du bundle portable (même architecture de dossiers que le
//! produit `OnionBit` pour cohabiter sur un même support amovible) :
//!
//! ```text
//! Explorer\
//!   Explorer.exe        lanceur (explorer-launcher → windows\)
//!   Explorer.portable   marqueur : sa présence fait vivre
//!                       `state/` à la racine du bundle
//!   windows\            payload OS : explorer_app.exe + DLLs
//!   state\              profil (historique, cookies, cache…) —
//!                       créé au premier lancement
//! ```
//!
//! La détection vit ici et non dans les binaires : le même
//! [`JsonFileStore::default_dir`](crate::JsonFileStore::default_dir)
//! sert le daemon (hôte REST) et le FFI (in-process).
//!
//! Invariant portable : **rien d'absolu n'est persisté sous
//! `state/`** — les fichiers du profil ne contiennent que du
//! contenu (URL, cookies, clés localStorage), jamais de chemin
//! machine : le bundle survit à un changement de lettre de
//! lecteur ou de point de montage.

use std::path::{Path, PathBuf};

/// Nom du répertoire de données canonique — **un seul dossier
/// à la racine du projet** (règle lead, étape 262 : rien en
/// `%APPDATA%`, rien en `%TEMP%`, aucun chemin choisi à la
/// main par les appelants — tout l'état des agents et des
/// vols vit dedans) :
///
/// ```text
/// <projet>/Explorer-data/
///   daemons/              annonces du registre (`daemon_list`)
///   profiles/<nom>/       profils standards (`daemon_spawn{profile:"nom"}`)
///     downloads/          téléchargements du profil
///   vols/<nom>/           workspace de vol : `profile/` +
///                         scripts, captures, rapports du vol
///   vols/                 corpus transverse (SYNTHESE.md…)
///   record/<nom>/         captures `EXPLORER_NET_RECORD` nues
///   logs/                 logs fichier (`EXPLORER_LOG`) +
///                         sentinelle de mort
///   crash/                dumps
/// ```
///
/// Résolution de [`data_root`], dans l'ordre :
/// `EXPLORER_DATA` → ancêtre (cwd, exe) portant déjà
/// `Explorer-data/` → racine du workspace dérivée du manifest
/// (binaires dev) → `None` hors contexte projet (l'appelant
/// retombe alors sur sa chaîne legacy : portable `state/`,
/// profil plateforme).
pub const DATA_DIR_NAME: &str = "Explorer-data";

/// Nom du marqueur à la racine d'un bundle portable — sa
/// présence seule commande le layout (copier l'exe hors du
/// bundle retombe sur le profil plateforme).
pub const PORTABLE_MARKER: &str = "Explorer.portable";

/// Sous-répertoire d'état sous la racine portable.
const PORTABLE_STATE_DIR: &str = "state";

/// Remonte depuis le dossier de `exe` jusqu'au marqueur
/// [`PORTABLE_MARKER`] → racine du bundle. La remontée est non
/// bornée : le marqueur porte le nom du produit — une fausse
/// positive supposerait un `Explorer.portable` déposé
/// volontairement sur un ancêtre.
#[must_use]
pub fn find_portable_root(exe: &Path) -> Option<PathBuf> {
    let mut dir = exe.parent();
    while let Some(d) = dir {
        if d.join(PORTABLE_MARKER).is_file() {
            return Some(d.to_path_buf());
        }
        dir = d.parent();
    }
    None
}

/// `state/` du bundle portable courant (`current_exe` →
/// marqueur), `None` hors bundle (dev, install classique).
#[must_use]
pub fn portable_state_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    find_portable_root(&exe).map(|r| r.join(PORTABLE_STATE_DIR))
}

/// `Explorer-data/` canonique — voir [`DATA_DIR_NAME`] pour
/// le contrat. `None` hors contexte projet : les appelants
/// gardent alors leur chaîne de repli (portable, plateforme).
#[must_use]
pub fn data_root() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("EXPLORER_DATA") {
        return Some(PathBuf::from(d));
    }
    std::env::current_dir()
        .ok()
        .and_then(|d| find_data_root(&d))
        .or_else(|| {
            std::env::current_exe()
                .ok()
                .and_then(|e| find_data_root(&e))
        })
        .or_else(workspace_data_root)
}

/// Premier ancêtre de `start` portant un `Explorer-data/`
/// existant — couvre les daemons lancés depuis une
/// sous-arborescence du projet ou un worktree frère.
fn find_data_root(start: &Path) -> Option<PathBuf> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        let c = d.join(DATA_DIR_NAME);
        if c.is_dir() {
            return Some(c);
        }
        dir = d.parent();
    }
    None
}

/// `Explorer-data/` sous la racine du workspace, dérivée de
/// `CARGO_MANIFEST_DIR` (`crates/<crate>` → deux niveaux) —
/// gardé par la présence de `Cargo.toml` à la racine : un
/// binaire prod copié sur une machine sans l'arbre de build
/// ne prétend pas au projet.
fn workspace_data_root() -> Option<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).ancestors().nth(2)?;
    root.join("Cargo.toml")
        .is_file()
        .then(|| root.join(DATA_DIR_NAME))
}

/// `<data>/daemons` — registre des daemons annoncés.
#[must_use]
pub fn daemons_dir() -> Option<PathBuf> {
    data_root().map(|r| r.join("daemons"))
}

/// `<data>/profiles` — racine des profils standards nommés.
#[must_use]
pub fn profiles_dir() -> Option<PathBuf> {
    data_root().map(|r| r.join("profiles"))
}

/// `<data>/profiles/<nom>` — `None` si le nom n'est pas un
/// nom propre (voir [`clean_name`]) ou hors contexte projet.
#[must_use]
pub fn profile_dir(name: &str) -> Option<PathBuf> {
    profiles_dir().and_then(|d| clean_name(name).map(|n| d.join(n)))
}

/// `<data>/vols` — corpus et workspaces des vols.
#[must_use]
pub fn vols_dir() -> Option<PathBuf> {
    data_root().map(|r| r.join("vols"))
}

/// `<data>/vols/<nom>` — workspace d'un vol.
#[must_use]
pub fn vol_dir(name: &str) -> Option<PathBuf> {
    vols_dir().and_then(|d| clean_name(name).map(|n| d.join(n)))
}

/// `<data>/record/<nom>` — captures réseau `EXPLORER_NET_RECORD`.
#[must_use]
pub fn record_dir(name: &str) -> Option<PathBuf> {
    data_root().and_then(|r| clean_name(name).map(|n| r.join("record").join(n)))
}

/// `<data>/logs` — logs fichier + sentinelle de mort.
#[must_use]
pub fn logs_dir() -> Option<PathBuf> {
    data_root().map(|r| r.join("logs"))
}

/// `<data>/crash` — dumps.
#[must_use]
pub fn crash_dir() -> Option<PathBuf> {
    data_root().map(|r| r.join("crash"))
}

/// Résolution d'un argument profil : `vol:<nom>` →
/// `vols/<nom>/profile`, `<nom>` nu → `profiles/<nom>` ;
/// `None` si l'argument est un chemin (séparateur/`:`) —
/// l'appelant décide alors s'il l'honore (flags/env explicites)
/// ou le refuse (surface agent : noms canoniques seulement).
#[must_use]
pub fn resolve_profile_name(arg: &str) -> Option<PathBuf> {
    if let Some(vol) = arg.strip_prefix("vol:") {
        return vol_dir(vol).map(|d| d.join("profile"));
    }
    profile_dir(arg)
}

/// Résolution d'un argument capture (`EXPLORER_NET_RECORD`/
/// `EXPLORER_NET_REPLAY`) : `<nom>` nu → `record/<nom>` sous
/// le projet ; chemin explicite → honoré tel quel (une env
/// posée à la main n'est pas du hardcodage).
#[must_use]
pub fn resolve_record_arg(arg: &str) -> PathBuf {
    record_dir(arg).unwrap_or_else(|| PathBuf::from(arg))
}

/// Nom propre de sous-répertoire : non vide, sans séparateur
/// ni `..` — garde-fou de la règle « rien ne sort de
/// `Explorer-data` » (un nom avec chemin = tentative de
/// sortie, rejetée par l'appelant).
fn clean_name(name: &str) -> Option<&str> {
    if name.is_empty()
        || name.contains(['/', '\\'])
        || name == "."
        || name == ".."
        || name.contains(':')
    {
        return None;
    }
    Some(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Dossier de test unique sous le temp dir système.
    fn unique_temp(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        std::env::temp_dir().join(format!(
            "explorer-paths-{tag}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn marqueur_detecte_depuis_le_payload() {
        let root = unique_temp("marker");
        let payload = root.join("windows");
        fs::create_dir_all(&payload).unwrap();
        fs::write(root.join(PORTABLE_MARKER), b"marqueur").unwrap();
        let exe = payload.join("explorer_app.exe");
        assert_eq!(find_portable_root(&exe), Some(root.clone()));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn sans_marqueur_pas_de_racine() {
        let root = unique_temp("none");
        let payload = root.join("windows");
        fs::create_dir_all(&payload).unwrap();
        let exe = payload.join("explorer_app.exe");
        assert_eq!(find_portable_root(&exe), None);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn marqueur_dossier_non_compte() {
        // Un répertoire nommé `Explorer.portable` (et non un
        // fichier) ne doit pas déclencher le mode portable.
        let root = unique_temp("dir");
        let payload = root.join("windows");
        fs::create_dir_all(&payload).unwrap();
        fs::create_dir_all(root.join(PORTABLE_MARKER)).unwrap();
        let exe = payload.join("explorer_app.exe");
        assert_eq!(find_portable_root(&exe), None);
        let _ = fs::remove_dir_all(&root);
    }
}
