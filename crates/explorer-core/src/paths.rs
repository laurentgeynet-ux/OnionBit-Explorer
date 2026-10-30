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
