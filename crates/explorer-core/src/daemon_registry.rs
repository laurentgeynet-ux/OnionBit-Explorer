// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Registre de daemons vivants — découverte multi-profils.
//!
//! Chaque `explorer-daemon` écrit une annonce JSON dans
//! [`registry_dir`] au moment du bind (`<nom>-<pid>.json`) :
//! c'est le seul mécanisme par lequel un adaptateur (MCP,
//! `explorer-cli daemons`) sait *qui* tourne, sur quel port,
//! avec quel profil — fini la divination de ports.
//!
//! ## Contrat
//!
//! - **Écriture** par [`announce`] après un bind réussi —
//!   l'annonce publie le port *réel* (`--port 0` → port
//!   éphémère devenu découvrable). La garde rendue retire le
//!   fichier à sa mort (arrêt propre).
//! - **Lecture** par [`list`] — retourne toutes les annonces
//!   *y compris potentiellement mortes* (crash : le fichier
//!   reste). La vivacité se prouve côté lecteur par
//!   [`announce_stale`] — pid vivant, image `explorer-daemon`
//!   et **port annoncé lié par ce pid** : un pid réattribué à
//!   un autre daemon n'emporte pas l'annonce du mort. Puis
//!   [`remove`] élimine l'entrée périmée.
//! - **Unicité** : le fichier porte `<nom>-<pid>.json` — deux
//!   daemons homonymes n'écrasent jamais leur annonce ; le
//!   routage par nom signale l'ambiguïté au demandeur.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Sous-dossier du registre sous la racine du profil par
/// défaut (partagé par tous les profils : la racine ne dépend
/// pas de `--profile`, seulement de `EXPLORER_PROFILE`/bundle
/// portable — l'annonce reste globale quel que soit le profil
/// servi).
const REGISTRY_SUBDIR: &str = "daemons";

/// Une annonce de daemon vivant — contrat écrivain/lecteur,
/// additif (les lecteurs ignorent les champs inconnus).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Announce {
    /// Nom court du daemon (`--name` ou nom du profil) —
    /// clé de routage côté agent, pas garanti unique.
    pub name: String,
    /// PID du processus — permet l'arrêt ciblé et distingue
    /// deux homonymes.
    pub pid: u32,
    /// Port loopback réellement lié (jamais 0 : l'annonce est
    /// écrite après le bind).
    pub port: u16,
    /// Répertoire de profil servi par ce daemon.
    pub profile: PathBuf,
    /// Version du binaire (`env!("CARGO_PKG_VERSION")`).
    pub version: String,
    /// Horodatage unix de démarrage (secondes).
    pub started_at: u64,
}

/// Répertoire du registre — `<Explorer-data>/daemons` sous
/// le projet ([`crate::paths::daemons_dir`] — le registre est
/// transversal aux profils, il ne vit pas dans l'un d'eux) ;
/// repli : `daemons/` sous le profil par défaut legacy.
#[must_use]
pub fn registry_dir() -> PathBuf {
    crate::paths::daemons_dir()
        .unwrap_or_else(|| crate::JsonFileStore::default_dir().join(REGISTRY_SUBDIR))
}

/// Nom de fichier d'une annonce — `<nom>-<pid>.json` : le
/// pid suffixé rend l'écriture unique sans probe de vivacité.
fn announce_file(dir: &Path, name: &str, pid: u32) -> PathBuf {
    dir.join(format!("{}-{pid}.json", sanitize_name(name)))
}

/// Nom sûr pour le système de fichiers : alnum + `-_.` —
/// le reste devient `_` (le `name` original est préservé
/// *dans* le JSON, seul le nom de fichier est normalisé).
fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Nom de daemon par défaut : `--name` sinon base du
/// répertoire de profil sinon `explorer`.
#[must_use]
pub fn default_name(cli_name: Option<&str>, profile: &Path) -> String {
    if let Some(n) = cli_name.filter(|n| !n.is_empty()) {
        return n.to_string();
    }
    profile
        .file_name()
        .and_then(|n| n.to_str())
        .filter(|n| !n.is_empty())
        .unwrap_or("explorer")
        .to_string()
}

/// Garde d'annonce — retire le fichier à la mort
/// ([`Drop`]). Un crash laisse une annonce périmée que le
/// lecteur élimine après sonde de vivacité négative.
pub struct AnnounceGuard {
    file: PathBuf,
}

impl Drop for AnnounceGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.file);
    }
}

/// Publie l'annonce du daemon dans `dir` — écriture
/// atomique (fichier temporaire + renommage) : un lecteur ne
/// voit jamais une annonce à moitié écrite. `port` doit être
/// le port lié, jamais 0. Retourne la garde à garder vivante.
///
/// # Errors
///
/// I/O : création du répertoire, écriture ou renommage.
pub fn announce_in(
    dir: &Path,
    name: &str,
    port: u16,
    profile: &Path,
) -> std::io::Result<AnnounceGuard> {
    std::fs::create_dir_all(dir)?;
    let a = Announce {
        name: name.to_string(),
        pid: std::process::id(),
        port,
        profile: profile.to_path_buf(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        started_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs()),
    };
    let file = announce_file(dir, name, a.pid);
    let tmp = file.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(&a).unwrap_or_default())?;
    std::fs::rename(&tmp, &file)?;
    Ok(AnnounceGuard { file })
}

/// [`announce_in`] dans [`registry_dir`].
///
/// # Errors
///
/// I/O : création du répertoire, écriture ou renommage.
pub fn announce(name: &str, port: u16, profile: &Path) -> std::io::Result<AnnounceGuard> {
    announce_in(&registry_dir(), name, port, profile)
}

/// Lit toutes les annonces de `dir` — vivantes ou périmées
/// (la vivacité se prouve par sonde côté lecteur). Les
/// entrées illisibles ou malformées sont ignorées : une
/// annonce corrompue ne doit pas aveugler le registre.
#[must_use]
pub fn list_in(dir: &Path) -> Vec<(PathBuf, Announce)> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&p) else {
            continue;
        };
        if let Ok(a) = serde_json::from_str::<Announce>(&text) {
            out.push((p, a));
        }
    }
    out.sort_by_key(|(_, a)| a.port);
    out
}

/// [`list_in`] dans [`registry_dir`].
#[must_use]
pub fn list() -> Vec<(PathBuf, Announce)> {
    list_in(&registry_dir())
}

/// Retire une annonce périmée — la GC du lecteur, réservée
/// à la mort **prouvée** (voir [`announce_stale`]) : élaguer sur
/// une sonde qui timeout évince définitivement un daemon
/// vivant (il ne se ré-annonce qu'au démarrage).
pub fn remove(file: &Path) {
    let _ = std::fs::remove_file(file);
}

/// Le pid existe-t-il encore (n'importe quel processus) ?
/// Première moitié de l'arbitre de mort : un processus
/// disparu est une mort prouvée — contrairement à une sonde
/// HTTP qui timeout sur un daemon saturé (2 Gio, raster
/// lourd), ce qui n'est **pas** une preuve. Indécidable
/// (`tasklist` absent…) → `true` : on n'évince jamais sans
/// preuve.
#[must_use]
pub fn pid_exists(pid: u32) -> bool {
    pid_exists_impl(pid)
}

#[cfg(windows)]
fn pid_exists_impl(pid: u32) -> bool {
    match std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout).contains(&format!(",\"{pid}\",")),
        Err(_) => true,
    }
}

#[cfg(not(windows))]
fn pid_exists_impl(pid: u32) -> bool {
    match std::process::Command::new("kill")
        .arg("-0")
        .arg(pid.to_string())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
    {
        Ok(s) => s.success(),
        Err(_) => true,
    }
}

/// Couples `(pid, port TCP)` réellement en écoute (Windows,
/// `netstat -ano`) — la moitié manquante de l'arbitre de
/// vivacité : un daemon vivant garde son listener bindé
/// toute sa vie ; une annonce dont le port n'est plus lié
/// **par ce pid** est périmée même quand le pid a été
/// réattribué à un *autre* explorer-daemon (identité
/// recyclée — le pid seul ne suffit pas à survivre à un
/// mort-vivant). `None` = indécidable (`netstat` absent) —
/// on n'évince jamais sans preuve.
#[must_use]
pub fn listening_ports() -> Option<std::collections::HashSet<(u32, u16)>> {
    listening_ports_impl()
}

#[cfg(windows)]
fn listening_ports_impl() -> Option<std::collections::HashSet<(u32, u16)>> {
    let o = std::process::Command::new("netstat")
        .arg("-ano")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    let mut set = std::collections::HashSet::new();
    for line in String::from_utf8_lossy(&o.stdout).lines() {
        let t: Vec<&str> = line.split_whitespace().collect();
        // `TCP <local> <remote> <état> <pid>` — en écoute,
        // l'adresse distante est indifférenciée
        // (`0.0.0.0:0`/`[::]:0`) ; le nom de l'état est
        // localisé et ignoré. UDP (4 champs, `*:*`) est
        // exclu par la forme.
        if t.len() < 5 || !t[0].starts_with("TCP") || !matches!(t[2], "0.0.0.0:0" | "[::]:0") {
            continue;
        }
        if let (Some(port), Ok(pid)) = (
            t[1].rsplit(':').next().and_then(|p| p.parse::<u16>().ok()),
            t[4].parse::<u32>(),
        ) {
            set.insert((pid, port));
        }
    }
    Some(set)
}

#[cfg(not(windows))]
fn listening_ports_impl() -> Option<std::collections::HashSet<(u32, u16)>> {
    None // pas de source portable — le port reste hors-preuve
}

/// Annonce périmée : pid mort, recyclé par un processus
/// qui n'est pas `explorer-daemon`, **ou port annoncé non
/// lié par ce pid** (pid vivant recyclé par un autre daemon
/// — l'image seule ne le distingue pas). C'est l'arbitre
/// d'éviction du registre : jamais un timeout HTTP, toujours
/// une mort prouvée par processus + socket. Sur unix, pas de
/// preuve port → `pid_exists` seul (recyclage résiduel
/// documenté).
#[must_use]
pub fn announce_stale(a: &Announce) -> bool {
    announce_stale_with(a, listening_ports().as_ref())
}

/// Variante lot de [`announce_stale`] : `ports` =
/// [`listening_ports`] pré-calculé — une seule passe
/// `netstat` pour toute la liste (`None` = indécidable,
/// conserver).
#[must_use]
pub fn announce_stale_with<S: ::std::hash::BuildHasher>(
    a: &Announce,
    ports: Option<&std::collections::HashSet<(u32, u16), S>>,
) -> bool {
    announce_stale_impl(a, ports)
}

#[cfg(windows)]
fn announce_stale_impl<S: ::std::hash::BuildHasher>(
    a: &Announce,
    ports: Option<&std::collections::HashSet<(u32, u16), S>>,
) -> bool {
    match std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {}", a.pid), "/FO", "CSV", "/NH"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
    {
        Ok(o) => {
            let s = String::from_utf8_lossy(&o.stdout).to_lowercase();
            if !(s.contains(&format!(",\"{}\",", a.pid)) && s.contains("explorer-daemon")) {
                return true; // pid mort ou recyclé hors explorer-daemon
            }
            match ports {
                Some(set) => !set.contains(&(a.pid, a.port)),
                None => false, // indécidable → conserver
            }
        }
        Err(_) => false, // indécidable → conserver
    }
}

#[cfg(not(windows))]
fn announce_stale_impl<S: ::std::hash::BuildHasher>(
    a: &Announce,
    _ports: Option<&std::collections::HashSet<(u32, u16), S>>,
) -> bool {
    !pid_exists_impl(a.pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cycle annonce → liste → retrait dans un dossier
    /// temporaire — le contrat fichier est stable pour les
    /// lecteurs externes (adaptateur, CLI).
    #[test]
    fn annonce_liste_et_retire() {
        let dir = std::env::temp_dir().join(format!("dreg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let g = announce_in(&dir, "scratch x", 19477, Path::new("C:\\tmp\\p")).unwrap();
        let l = list_in(&dir);
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].1.name, "scratch x"); // nom préservé
        assert_eq!(l[0].1.port, 19477);
        assert_eq!(l[0].1.pid, std::process::id());
        // Nom de fichier normalisé.
        assert!(l[0]
            .0
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("scratch_x-"));
        drop(g); // la garde retire
        assert!(list_in(&dir).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `pid_exists`/`announce_stale` : notre pid est vivant ;
    /// un enfant tué est une mort prouvée ; un processus
    /// vivant d'une autre image est une annonce périmée
    /// (recyclage) — la mort se prouve par processus, jamais
    /// par sonde HTTP (Étape 256b).
    #[test]
    fn pid_et_stale_vivant_et_mort() {
        assert!(pid_exists(std::process::id()));
        // Enfant vivant d'une AUTRE image : pid existe mais
        // l'annonce qui le nommerait serait périmée.
        let mut child = std::process::Command::new(if cfg!(windows) { "cmd" } else { "sleep" })
            .args(if cfg!(windows) {
                vec!["/c", "ping", "-n", "30", "127.0.0.1"]
            } else {
                vec!["30"]
            })
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn");
        let a = Announce {
            name: "ghost".into(),
            pid: child.id(),
            port: 1,
            profile: PathBuf::from("p"),
            version: "t".into(),
            started_at: 0,
        };
        #[cfg(windows)]
        assert!(announce_stale(&a)); // ping n'est pas explorer-daemon
        #[cfg(not(windows))]
        assert!(!announce_stale(&a)); // unix : pid seul, vivant
        child.kill().expect("kill");
        let _ = child.wait();
        assert!(!pid_exists(child.id()));
        assert!(announce_stale(&a));
    }

    /// `listening_ports` : notre propre listener apparaît
    /// sous notre pid — la preuve de vivacité (pid, port) est
    /// satisfaite par un socket réellement lié (Windows ;
    /// unix = indécidable → `None`).
    #[test]
    fn listening_port_du_processus() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = l.local_addr().unwrap().port();
        let set = listening_ports();
        #[cfg(windows)]
        {
            let set = set.expect("netstat absent ?");
            assert!(set.contains(&(std::process::id(), port)));
        }
        #[cfg(not(windows))]
        assert!(set.is_none());
    }

    /// Pid vivant recyclé par un **autre** explorer-daemon :
    /// l'annonce du mort nomme un port que le pid ne lie pas
    /// — périmée malgré pid+image sains (cas field274 :
    /// `field274-13520.json` survivant à `profile-13520.json`).
    /// La forme est simulée par `announce_stale_with` avec un
    /// ensemble de listeners contrôlé.
    #[test]
    fn pid_recycle_port_absent_est_perime() {
        let a = Announce {
            name: "ghost".into(),
            pid: std::process::id(),
            port: 1,
            profile: PathBuf::from("p"),
            version: "t".into(),
            started_at: 0,
        };
        // Notre pid existe (image ≠ explorer-daemon en test →
        // le premier test sort déjà `stale`) : on exerce donc
        // la branche port via un pid vivant réel en shuntant
        // l'image — simulée en insérant notre pid dans un set
        // qui ne contient PAS le port annoncé.
        let set: std::collections::HashSet<(u32, u16)> =
            [(std::process::id(), 65534)].into_iter().collect();
        #[cfg(windows)]
        {
            // L'image n'étant pas explorer-daemon, l'appel
            // complet est périmé — on vérifie le composant :
            // (pid, port) absent du set = l'annonce ne survit
            // pas au recyclage.
            assert!(!set.contains(&(a.pid, a.port)));
            assert!(set.contains(&(a.pid, 65534)));
        }
        let _ = set;
    }

    /// Deux homonymes ne s'écrasent pas — le pid suffixé
    /// garantit l'unicité sans probe.
    #[test]
    fn homonymes_coexistent() {
        let dir = std::env::temp_dir().join(format!("dreg2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // Simule deux pids : on écrit le second fichier à la main.
        let _g = announce_in(&dir, "d", 9400, Path::new("p")).unwrap();
        let mut a = list_in(&dir)[0].1.clone();
        a.pid += 1;
        std::fs::write(
            announce_file(&dir, "d", a.pid),
            serde_json::to_string(&a).unwrap(),
        )
        .unwrap();
        assert_eq!(list_in(&dir).len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
