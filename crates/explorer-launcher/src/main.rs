// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! `explorer-launcher` — lanceur portable du bundle Windows.
//!
//! `build_dist.ps1` copie ce petit binaire à la racine du bundle
//! sous le nom `Explorer.exe` ; il résout sa cible **relativement
//! à sa propre position** (`current_exe`) et lance
//! `windows\explorer_app.exe` :
//!
//! ```text
//! Explorer\
//!   Explorer.exe        ← ce lanceur
//!   Explorer.portable   marqueur (lu par le payload, pas ici)
//!   windows\            explorer_app.exe + DLLs + data\
//! ```
//!
//! Pourquoi pas des `.lnk` ? `WScript.Shell.CreateShortcut` grave
//! le **chemin absolu de la machine de build** dans `TargetPath` ;
//! Windows le réutilise tant qu'il existe — une copie du bundle
//! sur clé USB lancerait alors l'ancienne installation. Un exe
//! résolvant relativement est la convention `PortableApps` : aucun
//! chemin figé, aucune console parasite (sous-système GUI).
//!
//! Les arguments de la ligne de commande sont relayés tels quels.
//! Hors Windows : stub informatif — le crate reste dans le
//! workspace pour `cargo check` croisé.

#![forbid(unsafe_code)]
#![cfg_attr(windows, windows_subsystem = "windows")]

use std::path::Path;
use std::process::ExitCode;

/// Sous-répertoire du bundle contenant le payload de l'OS hôte
/// (le marqueur `Explorer.portable` est à côté, à la racine).
const PAYLOAD_DIR: &str = "windows";

/// Exécutable de l'application dans le payload — seule entrée du
/// bundle portable (mode FFI in-process : pas de daemon).
const UI_EXE: &str = "explorer_app.exe";

/// Résout la cible depuis la position du lanceur et la démarre
/// (`current_dir` = payload, args relayés) — sans attendre :
/// le lanceur rend la main immédiatement.
fn run() -> Result<(), String> {
    let self_exe =
        std::env::current_exe().map_err(|e| format!("chemin du lanceur illisible : {e}"))?;
    let dir = self_exe
        .parent()
        .ok_or_else(|| "lanceur sans dossier parent".to_string())?;
    launch(dir)
}

/// Corps de [`run`] paramétré par la racine (testable).
fn launch(dir: &Path) -> Result<(), String> {
    let payload = dir.join(PAYLOAD_DIR);
    let exe = payload.join(UI_EXE);
    if !exe.is_file() {
        return Err(format!("cible introuvable : {}", exe.display()));
    }
    std::process::Command::new(&exe)
        .args(std::env::args_os().skip(1))
        .current_dir(&payload)
        .spawn()
        .map_err(|e| format!("lancement de {} : {e}", exe.display()))?;
    Ok(())
}

/// Erreur visible — lanceur GUI sans console : une sortie stderr
/// serait invisible, d'où une boîte de dialogue native (safe,
/// via `rfd`).
#[cfg(windows)]
fn report_error(msg: &str) {
    rfd::MessageDialog::new()
        .set_title("Explorer")
        .set_description(format!(
            "{msg}\n\nRe-copiez le bundle Explorer complet (le dossier \
             « windows » doit rester à côté de ce lanceur)."
        ))
        .set_level(rfd::MessageLevel::Error)
        .show();
}

#[cfg(not(windows))]
fn report_error(msg: &str) {
    eprintln!("explorer-launcher : {msg}");
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            report_error(&msg);
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cible_sous_le_payload() {
        // Le chemin résolu reste relatif au lanceur — jamais
        // absolu figé au build (le point de l'exe vs .lnk).
        let dir = Path::new("E:\\cle-usb\\Explorer");
        let exe = dir.join(PAYLOAD_DIR).join(UI_EXE);
        assert!(exe.starts_with(dir));
        assert_eq!(exe.file_name().and_then(|n| n.to_str()), Some(UI_EXE));
    }

    #[test]
    fn cible_absente_erreur_explicite() {
        let dir =
            std::env::temp_dir().join(format!("explorer-launcher-test-{}", std::process::id()));
        let err = launch(&dir).unwrap_err();
        assert!(err.contains("cible introuvable"), "{err}");
    }
}
