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
//!
//! Étape 174 — téléchargements **directs**
//! (`Session::start_download` / `download_url` « enregistrer
//! sous », « enregistrer la source » média) : le fetch
//! `fetch_stream` et l'écriture `.part` → rename tournent
//! dans une tâche tokio **détachée** — le verrou session
//! n'est jamais tenu pendant le transfert (un gros fichier
//! ne gèle plus l'API, même discipline que
//! `navigate_shared`). L'état vivant est publié dans la map
//! partagée `active_downloads`, les étapes de vie arrivent
//! en `CoreEvent` (`DownloadStarted` → `DownloadProgress` →
//! `DownloadDone`/`DownloadFailed`).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use explorer_net::http::{FetchDest, Initiator, Request};
use explorer_net::policy::{referer_value_for, sec_fetch_headers};
use explorer_net::url::Url;

use super::net_items::disposition_filename;
use super::{
    collect_meta_csp, document_base, extract_title, meta_referrer_policy, meta_refresh,
    noscript_meta_refresh, CoreError, CoreEvent, DownloadInfo, DownloadState, NavDone, NavTail,
    Session, StreamDecoder, TabId,
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

/// État vivant d'un téléchargement direct en vol (étape
/// 174) — partagé entre `Session` (listing, annulation) et
/// la tâche détachée (réservation du nom, progression,
/// désinscription) via `active_downloads`, jamais par
/// re-verrou de la session.
pub(crate) struct ActiveDownload {
    /// Onglet émetteur (l'annulation vérifie la
    /// correspondance, même garde que `take_pending`).
    pub(crate) tab: TabId,
    /// URL demandée (la cible servie n'est connue
    /// qu'aux en-têtes — `resolved_url`).
    pub(crate) url: Url,
    /// Nom final réservé — `None` avant les en-têtes
    /// (`filename=` du `Content-Disposition` y participe).
    pub(crate) filename: Option<String>,
    /// Chemin cible final — réservé sous le verrou de la
    /// map (dédup disque + téléchargements concurrents) ;
    /// `Some` dès l'inscription quand `path` est explicite.
    pub(crate) target: Option<PathBuf>,
    /// Fichier temporaire `<cible>.part` en cours
    /// d'écriture — purgé à l'échec/annulation.
    pub(crate) tmp: Option<PathBuf>,
    /// Octets écrits à date.
    pub(crate) received: u64,
    /// `Content-Length` annoncé, si présent.
    pub(crate) total: Option<u64>,
    /// Type MIME déclaré (sans paramètres) — `""` avant
    /// les en-têtes.
    pub(crate) mime: String,
    /// Levier d'annulation (`cancel_download`) : `send`
    /// ou drop du `Sender` réveille la tâche, qui purge
    /// le `.part` et émet `DownloadFailed`.
    pub(crate) cancel: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Session {
    /// Répertoire de téléchargement par défaut
    /// (`accept_download` sans chemin explicite) — `None` =
    /// un chemin est exigé à l'acceptation.
    pub fn set_download_dir(&mut self, dir: Option<PathBuf>) {
        self.download_dir = dir;
    }

    /// Fiches des téléchargements **en attente** (offre
    /// étape 70 — `state: Pending`) et **en cours** (job
    /// détaché étape 174 — `state: Active`) de l'onglet.
    #[must_use]
    pub fn pending_downloads(&self, tab: TabId) -> Vec<DownloadInfo> {
        let mut out: Vec<DownloadInfo> = self
            .downloads
            .values()
            .filter(|d| d.tab == tab)
            .map(|d| DownloadInfo {
                id: d.id,
                tab: d.tab,
                url: d.url.to_string(),
                filename: suggested_filename(d),
                mime: d.mime.clone(),
                size: d.body.len(),
                state: DownloadState::Pending,
                total: d.size,
            })
            .collect();
        if let Ok(actives) = self.active_downloads.lock() {
            out.extend(
                actives
                    .iter()
                    .filter(|(_, a)| a.tab == tab)
                    .map(|(id, a)| DownloadInfo {
                        id: *id,
                        tab: a.tab,
                        url: a.url.to_string(),
                        filename: a
                            .filename
                            .clone()
                            .or_else(|| last_path_segment(&a.url))
                            .unwrap_or_else(|| "download".to_string()),
                        mime: a.mime.clone(),
                        size: usize::try_from(a.received).unwrap_or(usize::MAX),
                        state: DownloadState::Active,
                        total: a.total,
                    })
                    .collect::<Vec<_>>(),
            );
        }
        out
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
            state: DownloadState::Pending,
            total: dl.size,
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

    /// Démarre un téléchargement **direct** (étape 174 —
    /// « enregistrer sous », « enregistrer la source »
    /// média) : la requête repasse la pile transport
    /// complète — cookies, cache, HSTS, DNS — mais le fetch
    /// `fetch_stream` et l'écriture tournent dans une
    /// tâche tokio **détachée** ; la méthode rend
    /// l'identifiant immédiatement, le verrou session n'est
    /// jamais tenu pendant le transfert.
    ///
    /// Cycle de vie en `CoreEvent` : `DownloadStarted`
    /// (en-têtes reçus — nom réservé, `total` connu) →
    /// `DownloadProgress` par morceau → `DownloadDone`
    /// (`.part` renommé, journal `downloads.json` PR-1
    /// écrit) ou `DownloadFailed` (échec, borne
    /// `max_download_bytes` dépassée en vol, écriture
    /// refusée, annulation — `.part` purgé).
    ///
    /// La requête est enrichie comme une émission de la
    /// page courante : `initiator` `NavFrom(url de l'onglet)`
    /// — cookies `SameSite=Lax` admis sur GET (fidélité
    /// « enregistrer le lien sous », exception nav
    /// top-level), `Referer` sous la politique du document
    /// (`referrer_policy` de l'entrée) — plus `Sec-Fetch-*`
    /// document et `fetch_user` (geste explicite). Onglet
    /// vierge = `TopLevelNav`.
    ///
    /// `path` explicite écrit verbatim là (sans borne de
    /// nom) ; sinon le nom est résolu `hint` non vide →
    /// `filename=` du `Content-Disposition` → dernier
    /// segment d'URL → `download`, sanitisé et dédoublonné
    /// sous `download_dir`.
    ///
    /// Doit être appelée dans le contexte d'un runtime
    /// tokio (spawn de la tâche).
    ///
    /// # Errors
    /// [`CoreError::UnknownTab`] — onglet inconnu ;
    /// [`CoreError::Download`] — URL invalide, ni `path`
    /// ni `download_dir` configuré, ou appel hors runtime
    /// tokio.
    pub fn start_download(
        &mut self,
        tab: TabId,
        url: &str,
        path: Option<PathBuf>,
        hint: Option<String>,
    ) -> Result<u64, CoreError> {
        self.tabs.get(&tab).ok_or(CoreError::UnknownTab(tab))?;
        if path.is_none() && self.download_dir.is_none() {
            return Err(CoreError::Download(
                "aucun répertoire configuré — chemin explicite requis".to_string(),
            ));
        }
        let Ok(u) = Url::parse(url) else {
            return Err(CoreError::Download("URL invalide".to_string()));
        };
        if tokio::runtime::Handle::try_current().is_err() {
            return Err(CoreError::Download(
                "pas de runtime tokio (tâche de téléchargement impossible)".to_string(),
            ));
        }
        let req = self.download_request(tab, u.clone());
        let id = self.next_download_id;
        self.next_download_id += 1;
        let (cancel_tx, cancel_rx) = tokio::sync::oneshot::channel();
        let actives = Arc::clone(&self.active_downloads);
        {
            let mut m = self
                .active_downloads
                .lock()
                .map_err(|_| CoreError::Download("registre empoisonné".to_string()))?;
            m.insert(
                id,
                ActiveDownload {
                    tab,
                    url: u,
                    filename: None,
                    target: path,
                    tmp: None,
                    received: 0,
                    total: None,
                    mime: String::new(),
                    cancel: Some(cancel_tx),
                },
            );
        }
        tokio::spawn(download_job(DownloadArgs {
            id,
            tab,
            req,
            transport: Arc::clone(&self.transport),
            events: self.events.clone(),
            store: self.store.clone(),
            actives,
            dir: self.download_dir.clone(),
            hint,
            max_bytes: self.max_download_bytes,
            cancel_rx,
        }));
        Ok(id)
    }

    /// Variante compat `&mut self` de [`Self::start_download`]
    /// (FFI, CLI, tests) : abonne le bus **avant** le départ
    /// puis attend `DownloadDone`/`DownloadFailed` de l'id —
    /// un seul chemin réseau (la tâche détachée). `self` reste
    /// emprunté pendant toute la durée du transfert : le
    /// verrou `Mutex<Session>` de l'appelant partagé reste
    /// libre (c'est l'appelant exclusif qui attend, pas la
    /// session).
    ///
    /// # Errors
    /// Mêmes que [`Self::start_download`], plus
    /// [`CoreError::Download`] à l'échec rapporté par
    /// `DownloadFailed` ou si le bus d'événements se ferme.
    pub async fn download_url(
        &mut self,
        tab: TabId,
        url: &str,
        path: Option<PathBuf>,
        hint: Option<String>,
    ) -> Result<PathBuf, CoreError> {
        let mut rx = self.subscribe();
        let id = self.start_download(tab, url, path, hint)?;
        loop {
            match rx.recv().await {
                Ok(CoreEvent::DownloadDone { id: d, path: p, .. }) if d == id => {
                    return Ok(PathBuf::from(p));
                }
                Ok(CoreEvent::DownloadFailed { id: d, error, .. }) if d == id => {
                    return Err(CoreError::Download(error));
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    return Err(CoreError::Download("bus d'événements fermé".to_string()));
                }
            }
        }
    }

    /// Annule un téléchargement direct en cours (étape 174)
    /// : la tâche purge son `.part` et émet
    /// `DownloadFailed{annulé}`. Les offres `Pending`
    /// n'entrent pas ici — elles se rejettent par
    /// [`Self::decline_download`].
    ///
    /// # Errors
    /// [`CoreError::Download`] — identifiant inconnu ou
    /// d'un autre onglet.
    pub fn cancel_download(&mut self, tab: TabId, id: u64) -> Result<(), CoreError> {
        let mut m = self
            .active_downloads
            .lock()
            .map_err(|_| CoreError::Download("registre empoisonné".to_string()))?;
        match m.get_mut(&id) {
            Some(a) if a.tab == tab => {
                if let Some(tx) = a.cancel.take() {
                    let _ = tx.send(());
                }
                Ok(())
            }
            Some(_) => Err(CoreError::Download(format!(
                "identifiant d'un autre onglet : {id}"
            ))),
            None => Err(CoreError::Download(format!("identifiant inconnu : {id}"))),
        }
    }

    /// Journal des téléchargements terminés du profil
    /// (PR-1) — `[]` sans store configuré (mémoire pure).
    #[must_use]
    pub fn download_journal(&self) -> Vec<crate::DownloadRecord> {
        self.store.as_ref().map_or_else(Vec::new, |s| s.downloads())
    }

    /// Requête d'un téléchargement direct (étape 174) —
    /// même enrichissement top-level que `nav_begin` :
    /// `initiator` `NavFrom(page)` (cookies `SameSite=Lax`
    /// admis — l'exception navigation top-level GET colle
    /// au modèle « enregistrer le lien sous » des
    /// navigateurs), `dest=document`, `fetch_user` (geste
    /// explicite, `sec-fetch-user: ?1` émis même sous
    /// `NavFrom`), `Accept`/`Upgrade-Insecure-Requests`
    /// d'empreinte document, `Referer` sous la politique
    /// effective du document courant, graines de
    /// consentement CMP semées.
    fn download_request(&mut self, tab: TabId, url: Url) -> Request {
        let mut req = Request::get(url);
        let page = self.tabs.get(&tab).and_then(|t| t.url.clone());
        req.initiator = Some(
            page.clone()
                .map_or(Initiator::TopLevelNav, Initiator::NavFrom),
        );
        req.fetch_dest = FetchDest::Document;
        req.fetch_user = true;
        req.headers.extend(sec_fetch_headers(&req));
        if !req
            .headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("accept"))
        {
            req.headers.push((
                "accept".to_string(),
                "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8"
                    .to_string(),
            ));
        }
        if !req
            .headers
            .iter()
            .any(|(n, _)| n.eq_ignore_ascii_case("upgrade-insecure-requests"))
        {
            req.headers
                .push(("upgrade-insecure-requests".to_string(), "1".to_string()));
        }
        req.referrer_policy = self.tabs.get(&tab).and_then(|e| e.referrer_policy);
        if let Some(Initiator::NavFrom(p)) = &req.initiator {
            if !req.no_referer
                && !req
                    .headers
                    .iter()
                    .any(|(n, _)| n.eq_ignore_ascii_case("referer"))
            {
                if let Some(r) =
                    referer_value_for(p, &req.url, req.referrer_policy.unwrap_or_default())
                {
                    req.headers.push(("referer".to_string(), r));
                }
            }
        }
        self.seed_consent_cookies(&req);
        req
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
                // Journal de profil (PR-1) — le store relativise
                // le chemin sous `state/` (invariant portable).
                if let Some(s) = &self.store {
                    s.record_download(&crate::DownloadRecord {
                        url: dl.url.to_string(),
                        filename: suggested_filename(&dl),
                        path: target.display().to_string(),
                        size: dl.body.len() as u64,
                        finished_unix: std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map_or(0, |d| d.as_secs()),
                    });
                }
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

/// `dir/name` libre à la fois **sur disque** et dans les
/// cibles réservées des autres téléchargements actifs —
/// appelée sous le verrou de la map : la réservation est
/// atomique vis-à-vis des jobs concurrents (deux
/// `file.txt` simultanés → `file.txt` + `file-2.txt`).
fn reserve_target(
    actives: &std::sync::Mutex<HashMap<u64, ActiveDownload>>,
    id: u64,
    dir: &Path,
    name: &str,
) -> PathBuf {
    let taken = |m: &HashMap<u64, ActiveDownload>, p: &PathBuf| {
        p.exists() || m.values().any(|a| a.target.as_ref() == Some(p))
    };
    // Empoisonné : repli `unique_path` seul (collision
    // concurrente possible mais jamais de panic — la
    // dégradation est celle d'avant la réservation).
    let Ok(mut m) = actives.lock() else {
        return unique_path(dir, name);
    };
    let candidate = dir.join(name);
    let target = if taken(&m, &candidate) {
        let (stem, ext) = name.rsplit_once('.').map_or((name, ""), |(s, e)| (s, e));
        let mut found = None;
        for i in 2..=9_999u32 {
            let c = if ext.is_empty() {
                dir.join(format!("{stem}-{i}"))
            } else {
                dir.join(format!("{stem}-{i}.{ext}"))
            };
            if !taken(&m, &c) {
                found = Some(c);
                break;
            }
        }
        found.unwrap_or(candidate)
    } else {
        candidate
    };
    if let Some(a) = m.get_mut(&id) {
        a.target = Some(target.clone());
    }
    target
}

/// `<cible>.part` — fichier temporaire d'écriture (étape
/// 174) : le rename final n'expose jamais un corps
/// tronqué sous le nom attendu ; un `.part` orphelin de
/// crash est écrasé au prochain téléchargement du même nom.
fn part_path(target: &Path) -> PathBuf {
    let mut s = target.as_os_str().to_os_string();
    s.push(".part");
    PathBuf::from(s)
}

/// Charge utile du job détaché [`download_job`] (étape
/// 174) — tous `Arc`/clones : aucun accès `Session` en vol.
struct DownloadArgs {
    /// Identifiant alloué à l'inscription.
    id: u64,
    /// Onglet émetteur (events + annulation).
    tab: TabId,
    /// Requête enrichie (`download_request`).
    req: Request,
    /// Port réseau cloné.
    transport: Arc<dyn explorer_net::http::HttpTransport>,
    /// Bus d'événements cloné — `send` sans verrou.
    events: tokio::sync::broadcast::Sender<CoreEvent>,
    /// Store profil cloné (journal `downloads.json`) —
    /// `None` en mémoire pure.
    store: Option<Arc<dyn crate::ProfileStore>>,
    /// Registre partagé `id → ActiveDownload`.
    actives: Arc<std::sync::Mutex<HashMap<u64, ActiveDownload>>>,
    /// `download_dir` de session — ignoré si `req`
    /// porte une cible explicite (`target` déjà `Some`).
    dir: Option<PathBuf>,
    /// Nom suggéré par l'appelant (avant `filename=` CD).
    hint: Option<String>,
    /// Borne `max_download_bytes` — vérifiée au fil de
    /// l'eau (abort + purge `.part`).
    max_bytes: usize,
    /// Levier d'annulation — `send` ou drop du `Sender`
    /// (session morte) réveillent le job.
    cancel_rx: tokio::sync::oneshot::Receiver<()>,
}

/// Clôture d'un job en échec : purge le `.part` connu,
/// désinscrit l'entrée (la réservation de nom est rendue)
/// et émet `DownloadFailed`. Jamais de panic — best-effort.
async fn download_fail(a: &DownloadArgs, tmp: Option<&Path>, err: String) {
    if let Some(t) = tmp {
        let _ = tokio::fs::remove_file(t).await;
    }
    if let Ok(mut m) = a.actives.lock() {
        m.remove(&a.id);
    }
    let _ = a.events.send(CoreEvent::DownloadFailed {
        tab: a.tab,
        id: a.id,
        error: err,
    });
}

/// Job détaché d'un téléchargement direct (étape 174) :
/// `fetch_stream` → écriture incrémentale `<cible>.part` →
/// rename atomique → journal + `DownloadDone`. Chaque
/// étape est bornée ou best-effort — un échec se solde par
/// `DownloadFailed`, jamais par panic ni verrou Session.
async fn download_job(mut a: DownloadArgs) {
    // Fetch — annulable dès l'attente des en-têtes.
    let resp = tokio::select! {
        _ = &mut a.cancel_rx => {
            download_fail(&a, None, "annulé".to_string()).await;
            return;
        }
        r = a.transport.fetch_stream(&a.req) => r,
    };
    let mut resp = match resp {
        Ok(r) => r,
        Err(e) => {
            download_fail(&a, None, format!("{e}")).await;
            return;
        }
    };
    // Cible servie (post-redirections) — même repli que
    // `NavDone.url`.
    let served = resp
        .resolved_url
        .clone()
        .unwrap_or_else(|| a.req.url.clone());
    let total = resp
        .header("content-length")
        .and_then(|v| v.trim().parse::<u64>().ok());
    let mime = resp
        .header("content-type")
        .and_then(|v| v.split(';').next())
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    // Nom : `hint` non vide → `filename=` CD → segment
    // d'URL → `download` (ordre acté à la revue).
    let raw_name = a
        .hint
        .clone()
        .filter(|h| !h.is_empty())
        .or_else(|| {
            resp.header("content-disposition")
                .and_then(disposition_filename)
        })
        .or_else(|| last_path_segment(&served))
        .unwrap_or_else(|| "download".to_string());
    let filename = sanitize_filename(&raw_name);
    // Cible : `path` explicite (réservé à l'inscription,
    // verbatim) ; sinon nom réservé sous le verrou de la
    // map dans `download_dir`. Échec déjà émis → `None`.
    let Some((target, tmp, mut file)) = download_target(&a, &filename).await else {
        return;
    };
    let filename = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or(filename);
    // État + `DownloadStarted` — nom/taille désormais connus.
    if let Ok(mut m) = a.actives.lock() {
        if let Some(e) = m.get_mut(&a.id) {
            e.filename = Some(filename.clone());
            e.target = Some(target.clone());
            e.tmp = Some(tmp.clone());
            e.total = total;
            e.mime = mime;
            e.url = served.clone();
        }
    }
    let _ = a.events.send(CoreEvent::DownloadStarted {
        tab: a.tab,
        id: a.id,
        url: served.to_string(),
        filename,
        total,
    });
    let Some(received) = download_stream(&mut a, &mut resp, &mut file, &tmp, total).await else {
        return;
    };
    drop(file);
    download_commit(&a, &tmp, target, &served, received).await;
}

/// Phase cible de [`download_job`] : résout la destination
/// (`target` explicite réservée à l'inscription, sinon nom
/// dédupliqué sous `download_dir`), crée l'arbre parent et
/// ouvre `<cible>.part`. `Some(cible, tmp, fichier)` ou
/// `None` (échec déjà émis par `download_fail`).
async fn download_target(
    a: &DownloadArgs,
    filename: &str,
) -> Option<(PathBuf, PathBuf, tokio::fs::File)> {
    let target = a
        .actives
        .lock()
        .ok()
        .and_then(|m| m.get(&a.id).and_then(|e| e.target.clone()))
        .or_else(|| {
            a.dir
                .as_ref()
                .map(|d| reserve_target(&a.actives, a.id, d, filename))
        });
    let Some(target) = target else {
        download_fail(a, None, "aucun répertoire configuré".to_string()).await;
        return None;
    };
    let tmp = part_path(&target);
    if let Some(parent) = target.parent() {
        if let Err(e) = tokio::fs::create_dir_all(parent).await {
            download_fail(a, None, format!("répertoire : {e}")).await;
            return None;
        }
    }
    match tokio::fs::File::create(&tmp).await {
        Ok(f) => Some((target, tmp, f)),
        Err(e) => {
            download_fail(a, None, format!("écriture : {e}")).await;
            None
        }
    }
}

/// Phase corps de [`download_job`] : chunks → `.part`,
/// borne `max_download_bytes` au fil de l'eau (abort +
/// purge), `DownloadProgress` par chunk, annulation à
/// chaque `recv`. `Some(octets)` ou `None` (échec déjà
/// émis par `download_fail`).
async fn download_stream(
    a: &mut DownloadArgs,
    resp: &mut explorer_net::http::StreamedResponse,
    file: &mut tokio::fs::File,
    tmp: &Path,
    total: Option<u64>,
) -> Option<u64> {
    use tokio::io::AsyncWriteExt;
    let mut received = 0u64;
    loop {
        let chunk = tokio::select! {
            _ = &mut a.cancel_rx => {
                download_fail(a, Some(tmp), "annulé".to_string()).await;
                return None;
            }
            item = resp.body.recv() => item,
        };
        match chunk {
            Some(Ok(c)) => {
                received = received.saturating_add(c.len() as u64);
                if received > a.max_bytes as u64 {
                    download_fail(
                        a,
                        Some(tmp),
                        format!("corps trop volumineux (> {} octets)", a.max_bytes),
                    )
                    .await;
                    return None;
                }
                if let Err(e) = file.write_all(&c).await {
                    download_fail(a, Some(tmp), format!("écriture : {e}")).await;
                    return None;
                }
                if let Ok(mut m) = a.actives.lock() {
                    if let Some(e) = m.get_mut(&a.id) {
                        e.received = received;
                    }
                }
                let _ = a.events.send(CoreEvent::DownloadProgress {
                    tab: a.tab,
                    id: a.id,
                    received,
                    total,
                });
            }
            Some(Err(e)) => {
                download_fail(a, Some(tmp), format!("lecture : {e}")).await;
                return None;
            }
            None => break,
        }
    }
    if let Err(e) = file.flush().await {
        download_fail(a, Some(tmp), format!("écriture : {e}")).await;
        return None;
    }
    Some(received)
}

/// Phase commit de [`download_job`] : rename `.part` →
/// cible (TOCTOU : re-réserve un nom libre si la cible
/// apparaît entre réservation et commit — 3 retries),
/// journal profil, désinscription, `DownloadDone`.
async fn download_commit(
    a: &DownloadArgs,
    tmp: &Path,
    mut target: PathBuf,
    served: &Url,
    received: u64,
) {
    // Notre propre entrée rend le nom courant « pris » :
    // `reserve_target` choisit la variante `-2` suivante
    // (le `.part` garde son nom d'origine — `rename` le
    // fait basculer sur la nouvelle cible quel que soit
    // son nom).
    let mut attempts = 0u8;
    loop {
        match tokio::fs::rename(tmp, &target).await {
            Ok(()) => break,
            Err(e) => {
                let retriable = attempts < 3
                    && target.exists()
                    && matches!(
                        (
                            target.parent().map(Path::to_path_buf),
                            target.file_name().map(|n| n.to_string_lossy().into_owned())
                        ),
                        (Some(_), Some(_))
                    );
                if !retriable {
                    download_fail(a, Some(tmp), format!("rename : {e}")).await;
                    return;
                }
                attempts += 1;
                if let (Some(dir), Some(name)) = (
                    target.parent().map(Path::to_path_buf),
                    target.file_name().map(|n| n.to_string_lossy().into_owned()),
                ) {
                    target = reserve_target(&a.actives, a.id, &dir, &name);
                }
            }
        }
    }
    if let Some(s) = &a.store {
        s.record_download(&crate::DownloadRecord {
            url: served.to_string(),
            filename: target
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            path: target.display().to_string(),
            size: received,
            finished_unix: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
        });
    }
    if let Ok(mut m) = a.actives.lock() {
        m.remove(&a.id);
    }
    let _ = a.events.send(CoreEvent::DownloadDone {
        tab: a.tab,
        id: a.id,
        path: target.display().to_string(),
    });
}
