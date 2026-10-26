// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Téléchargements (étape 70) : une réponse top-level non
//! affichable (`Content-Disposition: attachment`,
//! `application/octet-stream`, type inconnu) est bufferisée
//! par [`NavJob::run`] puis enregistrée ici en attente de
//! décision — l'UI accepte (écriture fichier via
//! [`Session::accept_download`]) ou rejette
//! ([`Session::decline_download`] → le corps est rejoué
//! comme document, la dégradation « dump texte »
//! historique).
//!
//! La navigation qui produit un téléchargement **ne
//! commette rien** : pas d'URL, pas d'historique — la page
//! courante reste (modèle navigateur : un lien vers un
//! `attachment` ne quitte pas la page).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use explorer_net::url::Url;

use super::{
    collect_meta_csp, document_base, extract_title, meta_referrer_policy, meta_refresh,
    noscript_meta_refresh, CoreError, CoreEvent, DownloadInfo, NavDone, NavTail, Session,
    StreamDecoder, TabId,
};

/// Téléchargement bufferisé en attente de décision —
/// corps conservé en session (borné par
/// `max_download_bytes`), identifiant unique réservé à
/// `nav_begin`.
pub(crate) struct PendingDownload {
    /// Queue de navigation — epoch de garde, `push` et
    /// requête d'origine, rejoués par `decline_download`
    /// (commit « dump document » complet).
    pub(crate) tail: NavTail,
    /// Identifiant réservé à `nav_begin` (unique, non
    /// nécessairement consécutif — une nav documentaire
    /// consomme aussi un id).
    pub(crate) id: u64,
    /// Onglet de la navigation.
    pub(crate) tab: TabId,
    /// URL servie (post-redirection).
    pub(crate) url: Url,
    /// `filename=` du `Content-Disposition`, si déclaré.
    pub(crate) filename_hint: Option<String>,
    /// Type MIME déclaré (sans paramètres).
    pub(crate) mime: String,
    /// `Content-Length` annoncé, si présent.
    pub(crate) size: Option<u64>,
    /// Corps reçu (borné).
    pub(crate) body: Vec<u8>,
}

impl Session {
    /// Répertoire de téléchargement par défaut
    /// (`accept_download` sans chemin explicite) — `None` =
    /// un chemin est exigé à l'acceptation.
    pub fn set_download_dir(&mut self, dir: Option<PathBuf>) {
        self.download_dir = dir;
    }

    /// Fiches des téléchargements en attente (tous onglets
    /// ou filtré côté API).
    #[must_use]
    pub fn pending_downloads(&self, tab: TabId) -> Vec<DownloadInfo> {
        self.downloads
            .values()
            .filter(|d| d.tab == tab)
            .map(|d| DownloadInfo {
                id: d.id,
                tab: d.tab,
                url: d.url.to_string(),
                filename: suggested_filename(d),
                mime: d.mime.clone(),
                size: d.body.len(),
            })
            .collect()
    }

    /// Commit d'un corps non affichable (étape 70) : la nav
    /// n'écrit **rien** d'autre — URL, historique, DOM et
    /// titre restent sur la page courante. `Some(())` =
    /// enregistré + `DownloadOffered` émis ; `None` = epoch
    /// supplantée (la livraison est jetée proprement).
    pub(crate) fn offer_download(&mut self, tab: TabId, dl: PendingDownload) -> Option<()> {
        {
            let entry = self.tabs.get(&tab)?;
            if entry.nav_epoch != dl.tail.epoch {
                return None; // navigation supplantée — jetée.
            }
        }
        let info = DownloadInfo {
            id: dl.id,
            tab,
            url: dl.url.to_string(),
            filename: suggested_filename(&dl),
            mime: dl.mime.clone(),
            size: dl.body.len(),
        };
        self.downloads.insert(dl.id, dl);
        let _ = self.events.send(CoreEvent::DownloadOffered {
            tab,
            id: info.id,
            url: info.url,
            filename: info.filename,
            mime: info.mime,
            size: tail_size(&self.downloads, info.id),
        });
        Some(())
    }

    /// Accepte un téléchargement en attente : écrit le corps
    /// à `path` explicite ou sous `download_dir` avec le nom
    /// suggéré (sanitisé, unifié si collision), retire le
    /// pending et émet `DownloadDone`.
    ///
    /// # Errors
    /// [`CoreError::Download`] — identifiant inconnu ou
    /// d'un autre onglet, aucun répertoire configuré et pas
    /// de chemin explicite, ou écriture en échec.
    pub fn accept_download(
        &mut self,
        tab: TabId,
        id: u64,
        path: Option<PathBuf>,
    ) -> Result<PathBuf, CoreError> {
        let dl = self.take_pending(tab, id)?;
        let target = if let Some(p) = path {
            p
        } else if let Some(dir) = &self.download_dir {
            unique_path(dir, &suggested_filename(&dl))
        } else {
            // Ni chemin ni répertoire : le pending est remis
            // en place — l'offre reste décidable.
            self.downloads.insert(dl.id, dl);
            return Err(CoreError::Download(
                "aucun répertoire configuré — chemin explicite requis".to_string(),
            ));
        };
        if let Some(parent) = target.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                self.downloads.insert(dl.id, dl);
                return Err(CoreError::Download(format!("répertoire : {e}")));
            }
        }
        match std::fs::write(&target, &dl.body) {
            Ok(()) => {
                let _ = self.events.send(CoreEvent::DownloadDone {
                    tab,
                    id,
                    path: target.display().to_string(),
                });
                Ok(target)
            }
            Err(e) => {
                self.downloads.insert(dl.id, dl);
                Err(CoreError::Download(format!("écriture : {e}")))
            }
        }
    }

    /// Rejette un téléchargement : le corps bufferisé est
    /// **rejoué comme document** (décode + parse + commit
    /// complet) — la dégradation « dump texte » d'avant
    /// l'étape 70, volontairement conservée.
    ///
    /// # Errors
    /// [`CoreError::Download`] — identifiant inconnu ;
    /// erreurs de commit propagées (`nav_apply`).
    pub async fn decline_download(
        &mut self,
        tab: TabId,
        id: u64,
    ) -> Result<Option<String>, CoreError> {
        let dl = self.take_pending(tab, id)?;
        let mut dec = StreamDecoder::new(None);
        let mut html = dec.feed(&dl.body);
        html.push_str(&dec.finish());
        let mut tb = explorer_html::TreeBuilder::new();
        tb.push(&html);
        tb.finish();
        let dom = tb.into_dom();
        let title = extract_title(&dom);
        let (base_url, base_target) = document_base(&dom, &dl.url);
        // Même calcul `meta refresh` que `NavJob::run`
        // (CAP-143) : le rejeu « document » programme sa
        // navigation comme une réception réseau.
        let refresh_base = base_url.clone().unwrap_or_else(|| dl.url.clone());
        let refresh = meta_refresh(&dom)
            // Même repli `noscript` que `NavJob::run`
            // (vol google — fallback pur).
            .or_else(|| noscript_meta_refresh(&dom))
            .and_then(|(secs, raw)| {
                let target = match raw {
                    Some(r) => Url::resolve(&refresh_base, &r).ok()?,
                    None => dl.url.clone(),
                };
                std::time::Duration::try_from_secs_f64(secs.max(0.0))
                    .ok()
                    .map(|d| (d, target))
            });
        let done = NavDone {
            // Rejeu « document » d'un corps bufferisé : les
            // en-têtes de la réponse d'origine ne sont pas
            // conservés par `PendingDownload` — la politique
            // `Referer` vient des seules `<meta name=referrer>`
            // (étape 132, borne assumée).
            referrer_policy: meta_referrer_policy(&dom),
            base_url,
            base_target,
            refresh,
            // CSP au rejeu (CAP-145) : les en-têtes de la
            // réponse ne sont pas conservés — méta seules
            // (même borne assumée que la politique Referer).
            csp: collect_meta_csp(&dom)
                .iter()
                .map(|v| explorer_net::csp::CspPolicy::parse(v, true))
                .collect(),
            // `Clear-Site-Data` (CAP-147) : les en-têtes de
            // la réponse ne sont pas conservés — même borne
            // assumée que la politique Referer/CSP.
            clear_cookies: false,
            url: dl.url,
            dom,
            html,
            title,
        };
        let Some(mut sub) = self.nav_apply(tab, dl.tail, done)? else {
            return Ok(None);
        };
        sub.run().await;
        self.sub_commit(tab, sub).await
    }

    /// Retire le pending après vérification d'onglet — un
    /// identifiant d'un autre onglet est remis en place
    /// (refus, pas vol).
    fn take_pending(&mut self, tab: TabId, id: u64) -> Result<PendingDownload, CoreError> {
        match self.downloads.remove(&id) {
            Some(d) if d.tab == tab => Ok(d),
            Some(d) => {
                self.downloads.insert(id, d);
                Err(CoreError::Download(format!(
                    "identifiant d'un autre onglet : {id}"
                )))
            }
            None => Err(CoreError::Download(format!("identifiant inconnu : {id}"))),
        }
    }
}

/// `Content-Length` du pending enregistré (Some) — `None`
/// si le serveur n'a rien annoncé.
fn tail_size(map: &HashMap<u64, PendingDownload>, id: u64) -> Option<u64> {
    map.get(&id).and_then(|d| d.size)
}

/// Nom de fichier suggéré : `filename=` du
/// `Content-Disposition`, sinon dernier segment d'URL non
/// vide, sinon `download` — toujours sanitisé (séparateurs
/// et `..` retirés — un serveur hostile ne peut pas écrire
/// hors du répertoire choisi).
fn suggested_filename(dl: &PendingDownload) -> String {
    let raw = dl
        .filename_hint
        .clone()
        .filter(|f| !f.is_empty())
        .or_else(|| last_path_segment(&dl.url))
        .unwrap_or_else(|| "download".to_string());
    sanitize_filename(&raw)
}

/// Dernier segment de chemin non vide de l'URL — `None`
/// si le chemin est vide ou racine (repli `download`).
fn last_path_segment(url: &Url) -> Option<String> {
    url.path()
        .rsplit('/')
        .find(|seg| !seg.is_empty())
        .map(ToString::to_string)
}

/// Nom sûr pour un fichier local : séparateurs, `..`,
/// caractères de contrôle et espaces de bord retirés.
fn sanitize_filename(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut prev_dot = false;
    for ch in raw.trim().chars() {
        let bad =
            matches!(ch, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || ch.is_control();
        if bad {
            continue;
        }
        // `..` n'est jamais reproduit (traversée hostile).
        if ch == '.' {
            if prev_dot {
                continue;
            }
            prev_dot = true;
        } else {
            prev_dot = false;
        }
        out.push(ch);
    }
    let out = out.trim_end_matches(['.', ' ']).to_string();
    if out.is_empty() {
        "download".to_string()
    } else {
        out
    }
}

/// `dir/name`, unifié en `name-2.ext`, `name-3.ext`… si le
/// fichier existe déjà (borne : 9999 essais).
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let target = dir.join(name);
    if !target.exists() {
        return target;
    }
    let (stem, ext) = name.rsplit_once('.').map_or((name, ""), |(s, e)| (s, e));
    for i in 2..=9_999u32 {
        let candidate = if ext.is_empty() {
            dir.join(format!("{stem}-{i}"))
        } else {
            dir.join(format!("{stem}-{i}.{ext}"))
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    target
}
