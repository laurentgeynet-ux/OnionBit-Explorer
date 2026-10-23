// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Types du scheduler de sous-ressources : jobs de fetch,
//! graphe de modules, livraisons, requêtes CORS/cookies.
use super::*;
use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use explorer_css::{
    media_matches, parse_stylesheet, AtKind, AtRule, FontSrc, ImportRule, Origin, Rule,
    StyleEngine, StyleEnv,
};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;
use explorer_js::JsEngine;
use explorer_net::cookies::{partition_key, CookieJar};
use explorer_net::error::NetResult;
use explorer_net::http::{
    FetchDest, FetchMode, HttpTransport, Initiator, Method, ReferrerPolicy, Request, Response,
    StreamedResponse,
};
use explorer_net::policy::{initiator_headers_policy, sec_fetch_headers};
use explorer_net::url::Url;
use explorer_render::decode_font;
use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio::sync::broadcast;

/// Plan de collecte d'un document (étape 50 ; `<iframe>`
/// ajouté à l'étape 128) — produit de
/// [`Session::plan_subresources`], consommé par `sub_jobs`.
pub(crate) struct SubPlan {
    /// Feuilles `<link rel=stylesheet>` résolues.
    pub(crate) sheets: Vec<Url>,
    /// Scripts collectés (`<script src>` + inline/modules).
    pub(crate) scripts: Vec<ScriptItem>,
    /// `<img>`/`poster` résolus et dédupliqués.
    pub(crate) imgs: Vec<Url>,
    /// `<iframe src>` résolus — navigations de
    /// sous-contexte (bornés `max_frames`) ; la
    /// `referrerpolicy` de l'élément voyage avec l'URL
    /// (étape 132).
    pub(crate) frames: Vec<(Url, Option<ReferrerPolicy>)>,
}

/// Routage d'une complétion de sous-ressource (étape 50).
pub(crate) enum SubTag {
    /// Slot feuille liée (ordre document).
    Css(usize),
    /// Slot script externe (ordre document).
    Script(usize),
    /// `<img>` — clé `Url::as_str()` : la table images ne
    /// prend des `NodeId` qu'après la resynchro DOM.
    Img(Url),
    /// `<iframe src>` (étape 128) — navigation d'un
    /// sous-contexte : la réponse est un document HTML,
    /// parsé en `Dom` propre au commit.
    Frame(Url),
}

/// Graphe de modules préfetché (étape 50) — `sources` :
/// `url → corps` ; `alias` : spécificateur brut ou nom
/// d'entrée (`inline:N`) → URL de résolution des
/// dépendances (la page pour les entrées inline).
pub(crate) struct ModuleGraph {
    pub(crate) sources: HashMap<String, String>,
    pub(crate) alias: HashMap<String, String>,
}

/// Future de peinture progressive (étape 53d) — bornée par
/// la durée de vie du `&Dom` emprunté au `TreeBuilder`.
pub(crate) type PaintFut<'a> = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>>;

/// Phase « fetch + parse » d'une navigation, détachable de
/// la session (étape 53d) : construite sous `&mut self` par
/// [`Session::nav_begin`] (epoch armée, `Arc` du transport
/// cloné), [`NavJob::run`] ne réclame aucun `&Session` —
/// un hôte à `Mutex<Session>` garde son API réactive
/// pendant tout le téléchargement. La peinture progressive
/// est livrée à un rappel injecté : chaque snapshot du DOM
/// en construction est consommé dans sa propre future
/// (re-lock court côté appelant partagé, `nav_paint`
/// direct côté `&mut self`).
pub(crate) struct NavJob {
    /// Onglet — `DownloadProgress`/`DownloadOffered` (70).
    pub(crate) tab: TabId,
    /// Epoch capturée à `nav_begin` — comparée à
    /// `Tab::nav_epoch` à chaque reprise du verrou.
    pub(crate) epoch: u64,
    /// Requête de navigation (GET/POST).
    pub(crate) req: Request,
    /// `push` du commit (historique).
    pub(crate) push: bool,
    /// Titre de l'onglet avant navigation — `TitleChanged`.
    pub(crate) prev_title: Option<String>,
    /// Transport cloné — fetch hors session.
    pub(crate) transport: Arc<dyn HttpTransport>,
    /// Cadence de peinture progressive.
    pub(crate) paint_chunk_bytes: usize,
    pub(crate) paint_interval: std::time::Duration,
    /// Bus d'événements — progression du buffer
    /// téléchargement émise hors verrou (étape 70).
    pub(crate) events: broadcast::Sender<CoreEvent>,
    /// Borne du corps bufferisé (étape 70) — dépassement =
    /// `Fetch` (page d'erreur 69), jamais de gonflage.
    pub(crate) max_download_bytes: usize,
    /// Compteur d'identifiants de téléchargement — cloné
    /// par `nav_begin`, incrémenté à la détection.
    pub(crate) next_download_id: u64,
}

/// Queue de [`NavJob::run`] — ce que le commit consomme
/// après le `self` du job (re-publié sans l'emprunt du
/// rappel de peinture).
pub(crate) struct NavTail {
    /// Epoch — garde [`Session::nav_apply`].
    pub(crate) epoch: u64,
    /// `push` historique.
    pub(crate) push: bool,
    /// Titre avant navigation — `TitleChanged`.
    pub(crate) prev_title: Option<String>,
    /// Requête d'origine — rejouée par `reload` et stockée
    /// dans l'entrée d'historique (étape 56, POST).
    pub(crate) req: Request,
}

/// Sortie de [`NavJob::run`] — document parsé prêt à poser.
pub(crate) struct NavDone {
    /// URL effectivement naviguée.
    pub(crate) url: Url,
    /// DOM du document complet.
    pub(crate) dom: Dom,
    /// HTML décodé (miroir JS à `nav_apply`).
    pub(crate) html: String,
    /// Titre extrait.
    pub(crate) title: Option<String>,
    /// Politique `Referer` du document (étape 132) —
    /// en-tête `Referrer-Policy:` de la réponse puis
    /// `<meta name=referrer>` (la meta, traitée après les
    /// en-têtes, l'emporte) ; `None` = défaut plateforme.
    pub(crate) referrer_policy: Option<ReferrerPolicy>,
}

/// Issue de [`NavJob::run`] (étape 70) : document à commettre
/// ou téléchargement bufferisé à proposer.
pub(crate) enum NavOutcome {
    /// Document affichable — commit `nav_apply` habituel.
    Document(NavDone, NavTail),
    /// Corps non affichable bufferisé — la session le garde
    /// en attente d'acceptation (`PendingDownload`, queue de
    /// navigation incluse pour le rejeu « dump document »
    /// d'un rejet).
    Download(super::downloads::PendingDownload),
}

/// Métadonnées de détection téléchargement (étape 70) —
/// extraites des en-têtes de la réponse top-level.
pub(crate) struct DownloadMeta {
    /// `filename=` du `Content-Disposition`, si déclaré.
    pub(crate) filename_hint: Option<String>,
    /// Type MIME déclaré (sans paramètres, minuscules).
    pub(crate) mime: String,
    /// `Content-Length` annoncé, si présent et valide.
    pub(crate) total: Option<u64>,
}

/// `Some(meta)` si la réponse top-level n'est **pas** un
/// document affichable (étape 70) : `Content-Disposition:
/// attachment`, `application/octet-stream`, ou type
/// inconnu du pipeline (`application/*` hors html/xhtml,
/// `image/*`, `audio/*`, `video/*`, `font/*`). `text/*`,
/// les XML de document et l'absence de type restent
/// traités comme document — le statu quo est la dégradation.
pub(crate) fn download_meta(headers: &[(String, String)]) -> Option<DownloadMeta> {
    let get = |name: &str| {
        headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    };
    let total = get("content-length").and_then(|v| v.trim().parse::<u64>().ok());
    let disposition = get("content-disposition").unwrap_or_default();
    let mime = get("content-type")
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    let filename_hint = disposition_filename(disposition);
    let attached = disposition
        .split(';')
        .next()
        .is_some_and(|d| d.trim().eq_ignore_ascii_case("attachment"));
    let doc_like = mime.is_empty()
        || mime == "text/html"
        || mime == "application/xhtml+xml"
        || mime.starts_with("text/")
        || mime.ends_with("+xml");
    if attached || !doc_like {
        return Some(DownloadMeta {
            filename_hint,
            mime: if mime.is_empty() {
                "application/octet-stream".to_string()
            } else {
                mime
            },
            total,
        });
    }
    None
}

/// `filename=` d'un `Content-Disposition` — guillemets et
/// espaces tolérés ; `filename*=` (RFC 5987) non décodé
/// en v1 (déviation documentée, le repli est le segment
/// d'URL).
fn disposition_filename(disposition: &str) -> Option<String> {
    for part in disposition.split(';').skip(1) {
        let Some((k, v)) = part.split_once('=') else {
            continue;
        };
        if k.trim().eq_ignore_ascii_case("filename") {
            let f = v.trim().trim_matches('"');
            if !f.is_empty() {
                return Some(f.to_string());
            }
        }
    }
    None
}

impl NavJob {
    /// Fetch + parse incrémental hors session : chaque
    /// morceau décodé alimente le tree-builder —
    /// `TreeBuilder::push` garantit un arbre cohérent à
    /// frontière de token (R-1) — et à la cadence
    /// configurée (N octets ou N ms) le DOM en construction
    /// est livré à `paint`. Aucun `&mut Session` ne
    /// traverse l'`await` : le verrou hôte reste libre
    /// pendant tout le téléchargement (étape 53d).
    pub(crate) async fn run<F>(&mut self, mut paint: F) -> Result<NavOutcome, CoreError>
    where
        F: for<'a> FnMut(&'a Dom) -> PaintFut<'a>,
    {
        if let Some(outcome) = self.about_blank() {
            return Ok(outcome);
        }
        let mut resp = self
            .transport
            .fetch_stream(&self.req)
            .await
            .map_err(|e| CoreError::Fetch(e.to_string()))?;
        if !(200..300).contains(&resp.status) {
            return Err(CoreError::HttpStatus(resp.status));
        }
        // Cible post-redirection (étape 67) : barre d'adresse,
        // historique, base des sous-ressources et fragment
        // `Location` (`/x#sec`) se commettent sur l'URL finale,
        // pas sur l'URL de requête.
        let resolved = resp
            .resolved_url
            .take()
            .unwrap_or_else(|| self.req.url.clone());
        // `Referrer-Policy:` de la réponse (étape 132) —
        // politique initiale du document ; une `<meta
        // name=referrer>` reconnue l'écrasera après parse.
        let header_policy = resp
            .headers
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case("referrer-policy"))
            .filter_map(|(_, v)| ReferrerPolicy::parse(v))
            .next_back();
        // Corps non affichable (étape 70) : bufferisé avec
        // borne et progression — jamais décodé en mojibake.
        if let Some(meta) = download_meta(&resp.headers) {
            return self.buffer_download(resp, resolved, meta).await;
        }
        let mut tb = TreeBuilder::new();
        // Charset du transport (`Content-Type`) en amorce du
        // décodeur — priorité sur le prescan `<meta>` (53e).
        let dec_hint = transport_charset(&resp.headers)
            .as_deref()
            .and_then(label_encoding);
        let mut dec = StreamDecoder::new(dec_hint);
        let mut html = String::new();
        let mut since_paint = 0usize;
        let mut last_paint = Instant::now();
        // Plancher adaptatif : chaque passe rejoue le pipeline
        // **complet** sur le DOM courant — sur un document
        // géant le coût grandit avec l'arbre (mesuré : jusqu'à
        // ~40 s/passe → burn quadratique de plusieurs minutes
        // qui affame le fetch en vol sur le runtime
        // `current_thread`). Une passe coûtant T exige désormais
        // ≥ T d'accumulation avant la suivante : sur petite
        // page les deux déclencheurs d'origine (chunk +
        // intervalle) restent libres (plancher = intervalle,
        // coût < intervalle), sur gros document le duty
        // cycle est borné (~50 %) — fin du burn O(n²).
        let mut paint_floor = self.paint_interval;
        while let Some(chunk) = resp.body.recv().await {
            let bytes = chunk.map_err(|e| CoreError::Fetch(e.to_string()))?;
            let text = dec.feed(&bytes);
            html.push_str(&text);
            tb.push(&text);
            since_paint += bytes.len();
            // Fenêtre de sniffing du décodeur ou morceau sans
            // texte décodable : le DOM n'a pas bougé — peindre
            // serait un no-op (et `since_paint` reste accumulé
            // pour déclencher dès le premier texte réel, 56c).
            if !text.is_empty()
                && (since_paint >= self.paint_chunk_bytes
                    || last_paint.elapsed() >= self.paint_interval)
                && last_paint.elapsed() >= paint_floor
            {
                let t = Instant::now();
                paint(tb.dom()).await;
                paint_floor = self.paint_interval.max(t.elapsed());
                since_paint = 0;
                last_paint = Instant::now();
            }
        }
        let text = dec.finish();
        html.push_str(&text);
        tb.push(&text);
        tb.finish();
        let dom = tb.into_dom();
        let title = extract_title(&dom);
        let done = NavDone {
            url: resolved,
            // `<meta name=referrer>` l'emporte sur
            // l'en-tête (traitée après lui, spec).
            referrer_policy: meta_referrer_policy(&dom).or(header_policy),
            dom,
            html,
            title,
        };
        let tail = NavTail {
            epoch: self.epoch,
            push: self.push,
            prev_title: self.prev_title.take(),
            req: self.req.clone(),
        };
        Ok(NavOutcome::Document(done, tail))
    }

    /// Corps non affichable (étape 70) : bufferisé avec
    /// borne et progression — jamais décodé en mojibake.
    /// `DownloadOffered` n'est émis qu'au commit
    /// (`offer_download`) : un « accept » de l'UI avant
    /// l'enregistrement du pending ne peut pas échouer.
    async fn buffer_download(
        &mut self,
        mut resp: StreamedResponse,
        resolved: Url,
        meta: DownloadMeta,
    ) -> Result<NavOutcome, CoreError> {
        let dl_id = self.next_download_id;
        let mut body = Vec::new();
        while let Some(chunk) = resp.body.recv().await {
            let bytes = chunk.map_err(|e| CoreError::Fetch(e.to_string()))?;
            if body.len() + bytes.len() > self.max_download_bytes {
                return Err(CoreError::Fetch("téléchargement hors bornes".to_string()));
            }
            body.extend_from_slice(&bytes);
            let _ = self.events.send(CoreEvent::DownloadProgress {
                tab: self.tab,
                id: dl_id,
                received: body.len() as u64,
                total: meta.total,
            });
        }
        Ok(NavOutcome::Download(super::downloads::PendingDownload {
            tail: NavTail {
                epoch: self.epoch,
                push: self.push,
                prev_title: self.prev_title.take(),
                req: self.req.clone(),
            },
            id: dl_id,
            tab: self.tab,
            url: resolved,
            filename_hint: meta.filename_hint,
            mime: meta.mime,
            size: meta.total,
            body,
        }))
    }

    /// `about:blank` (étape 88) : le document vide canonique
    /// ne se fetch pas — l'arbre squelette du tree-builder
    /// (html/head/body implicites à l'Eof) est commis tel
    /// quel. Tout autre `about:*` tombe sur le transport →
    /// page d'erreur, comme Chrome.
    fn about_blank(&mut self) -> Option<NavOutcome> {
        if self.req.url.scheme() != "about" || !matches!(self.req.url.path(), "" | "blank") {
            return None;
        }
        let mut tb = TreeBuilder::new();
        tb.finish();
        let tail = NavTail {
            epoch: self.epoch,
            push: self.push,
            prev_title: self.prev_title.take(),
            req: self.req.clone(),
        };
        Some(NavOutcome::Document(
            NavDone {
                url: self.req.url.clone(),
                dom: tb.into_dom(),
                html: String::new(),
                title: None,
                referrer_policy: None,
            },
            tail,
        ))
    }
}

/// Évaluation d'un module différée au commit (étape 53d) —
/// le graphe statique est déjà rempli (prefetch hors
/// session dans [`SubJob::run`]), le commit ne fait plus
/// que l'`eval_module` synchrone.
pub(crate) struct ModEval {
    /// Clé de registre moteur (URL de l'entrée ou
    /// `inline:N`).
    pub(crate) name: String,
    /// Graphe rempli — `sources` + `alias`.
    pub(crate) graph: ModuleGraph,
}

/// Phase « sous-ressources » d'une navigation, détachable
/// de la session (étape 53d) : possède le plan de collecte,
/// le moteur JS du document et les tampons de réception —
/// [`SubJob::run`] télécharge la vague CSS/scripts/images,
/// les `@font-face` et les graphes de modules sans
/// `&Session` ; [`Session::sub_commit`] applique.
pub(crate) struct SubJob {
    /// Onglet + epoch de la navigation.
    pub(crate) tab: TabId,
    pub(crate) epoch: u64,
    /// URL du document — base des sous-ressources et
    /// `Referer` de la politique réseau.
    pub(crate) url: Url,
    /// `push` historique + titre avant navigation.
    pub(crate) push: bool,
    pub(crate) prev_title: Option<String>,
    /// Requête d'origine — `last_req` du commit et entrée
    /// d'historique poussée (étape 56, rejoue un POST).
    pub(crate) req: Request,
    /// Titre extrait au parse.
    pub(crate) title: Option<String>,
    /// Transport cloné — fetch hors session.
    pub(crate) transport: Arc<dyn HttpTransport>,
    /// Bus d'événements — `PipelineError` des évaluations
    /// « arrival » pendant la vague.
    pub(crate) events: broadcast::Sender<CoreEvent>,
    /// URL de la page — politique mixed-content/`Referer`
    /// du prefetch modules.
    pub(crate) page: Url,
    /// Politique `Referer` du document commis (étape 132 —
    /// `Referrer-Policy:`/`<meta name=referrer>` connus à
    /// `nav_apply`) — gouverne toutes les sous-ressources
    /// sauf attribut `referrerpolicy=` par élément
    /// (`<iframe>` seul honoré v1).
    pub(crate) page_policy: Option<ReferrerPolicy>,
    /// Plan de collecte — URLs résolues à `nav_apply`.
    pub(crate) sheet_urls: Vec<Url>,
    pub(crate) scripts: Vec<ScriptItem>,
    pub(crate) img_urls: Vec<Url>,
    /// `<iframe src>` résolus à `nav_apply` (étape 128 —
    /// déjà dédupliqués/mixed-content-filtrés/bornés) ;
    /// la `referrerpolicy` de l'élément voyage avec l'URL
    /// (étape 132).
    pub(crate) frame_urls: Vec<(Url, Option<ReferrerPolicy>)>,
    /// Jar partagé — les `Set-Cookie` des documents
    /// `<iframe>` sont capturés à réception (étape 128) :
    /// même partitionnement CHIPS que `CookieTransport`
    /// (seau = site top-level de la page parente).
    pub(crate) cookies: Arc<CookieJar>,
    /// Corps JSON des `<script type="importmap">` (ordre
    /// document) — poussés au moteur avant le premier
    /// `eval_module` (issue #6).
    pub(crate) import_maps: Vec<String>,
    /// `imports` fusionné (`merge_import_maps`) — sert au
    /// prefetch : `module_deps` réécrit les bare specifiers
    /// avant `Url::resolve`, comme le moteur à l'eval.
    pub(crate) import_specs: Vec<(String, String)>,
    /// `<style>` inline du document — une entrée par
    /// élément (étape 104) — nourrit la résolution
    /// `@font-face` (le DOM n'est pas accessible hors
    /// session).
    pub(crate) doc_sheets: Vec<String>,
    /// Moteur du document — les `async` classiques sont
    /// évalués en ligne à réception pendant la vague.
    pub(crate) eng: Option<Box<dyn JsEngine>>,
    /// Réceptions de la vague — indexées par ordre document.
    pub(crate) css_slots: Vec<Option<String>>,
    pub(crate) script_srcs: Vec<Option<String>>,
    pub(crate) img_bodies: HashMap<String, Vec<u8>>,
    /// Documents `<iframe>` reçus et parsés (étape 128) —
    /// un `SubFrame` par fetch 2xx, appliqués à l'onglet au
    /// commit.
    pub(crate) frames: Vec<SubFrame>,
    /// Fonds `url(http…)` fetchés (étape 98) : `bg_aliases`
    /// relie le texte `url(...)` brut (déséchappé — la forme
    /// conservée en valeur calculée) à l'URL résolue, les
    /// corps vivent sous `bg_bodies` indexés par résolue.
    /// Le `NodeId` propriétaire est inconnu ici (le DOM est
    /// hors portée) : il sera retrouvé au relayout par la
    /// valeur `background-image` calculée.
    pub(crate) bg_aliases: HashMap<String, String>,
    pub(crate) bg_bodies: HashMap<String, Vec<u8>>,
    /// Masques `mask-image: url(http…)` fetchés (étape 121 —
    /// réponse à la demande PIPE R-86) : même chaîne que les
    /// fonds — texte `url(...)` brut → URL résolue. Les corps
    /// vivent dans `bg_bodies` (magasin partagé par résolue :
    /// une image servant de fond *et* de masque n'est
    /// fetchée/décodée qu'une fois). Pas de `NodeId` à
    /// retrouver : `PaintImages::register_mask` indexe par le
    /// texte brut — les pseudo-boîtes sont couvertes de fait.
    pub(crate) mask_aliases: HashMap<String, String>,
    /// Corps des modules `async` arrivés (ordre d'arrivée).
    pub(crate) async_mods: Vec<(Url, String)>,
    /// Faces `@font-face` téléchargées et décodées —
    /// `family`, `weight`, `italic`, données brutes.
    pub(crate) fonts: Vec<(String, u16, bool, Vec<u8>)>,
    /// Modules à évaluer au commit — arrival puis post.
    pub(crate) arrival_evals: Vec<ModEval>,
    pub(crate) post_evals: Vec<ModEval>,
    /// Viewport session recopié — les media queries
    /// (`@font-face`/`media=`) s'évaluent contre lui, pas
    /// le 1024×768 par défaut.
    pub(crate) viewport: (f32, f32),
    /// Schéma de couleurs réglé recopié (étape 97) — le
    /// filtre `media=`/`prefers-color-scheme` s'évalue
    /// contre le réglage session pendant la vague, hors
    /// verrou.
    pub(crate) color_scheme: explorer_css::ColorScheme,
    /// Bornes recopiées de la config session.
    pub(crate) max_inflight: usize,
    pub(crate) max_modules: usize,
    pub(crate) font_budget: usize,
    pub(crate) max_imports: usize,
    /// Budget fetchs d'images — partagé `<img>`/fonds CSS
    /// (étape 98) ; `img_urls` est déjà tronqué à cette
    /// borne au planning.
    pub(crate) max_images: usize,
}

/// Reliquat du commit de navigation entre la tête (sous
/// `&mut self`) et la queue (étape 90) — les champs que le
/// rendu détaché et la finalisation consomment encore :
/// `img_bodies` nourrit le décodage hors verrou, le reste
/// sert à `sub_commit_tail` (historique, `Navigated`,
/// fragment).
pub(crate) struct SubCommitTail {
    /// URL du document — base du matching `<img>` et
    /// comparaison `Navigated`.
    pub(crate) url: Url,
    /// `push` historique + titre avant navigation.
    pub(crate) push: bool,
    pub(crate) prev_title: Option<String>,
    /// Requête d'origine — entrée d'historique poussée.
    pub(crate) req: Request,
    /// Titre extrait au parse — valeur de retour du commit.
    pub(crate) title: Option<String>,
    /// Corps `<img>` de la vague — décodés pendant le rendu.
    pub(crate) img_bodies: HashMap<String, Vec<u8>>,
    /// Fonds `url(http…)` de la vague (étape 98) — posés sur
    /// la payload avant le relayout, insérés à la cascade.
    pub(crate) bg_aliases: HashMap<String, String>,
    pub(crate) bg_bodies: HashMap<String, Vec<u8>>,
    /// Masques `mask-image` de la vague (étape 121) — corps
    /// partagés dans `bg_bodies`.
    pub(crate) mask_aliases: HashMap<String, String>,
}

impl SubJob {
    /// Vague de sous-ressources + fontes + prefetch des
    /// graphes de modules — intégralement hors session :
    /// tous les tampons vivent dans le job, le commit les
    /// applique (étape 53d). Les scripts `async` classiques
    /// sont évalués à réception sur le moteur embarqué.
    pub(crate) async fn run(&mut self) {
        // Slots de réception indexés par ordre document ;
        // `slot_items[slot]` = index de l'item script.
        self.css_slots = vec![None; self.sheet_urls.len()];
        let slot_items: Vec<usize> = self
            .scripts
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.slot.map(|_| i))
            .collect();
        self.script_srcs = vec![None; slot_items.len()];

        let tab = self.tab;
        let events = self.events.clone();
        let eval_arrival = |eng: &mut Option<Box<dyn JsEngine>>, src: &str| {
            if let Some(e) = eng.as_mut() {
                if let Err(err) = e.eval(src) {
                    tracing::debug!(tab, error = %err, "script async en échec");
                    let _ = events.send(CoreEvent::PipelineError {
                        tab: Some(tab),
                        message: err.to_string(),
                    });
                }
            }
        };
        // Émission par priorité : CSS → scripts → images.
        let jobs = sub_jobs(
            &self.url,
            &self.sheet_urls,
            &self.scripts,
            &self.img_urls,
            &self.frame_urls,
            self.page_policy,
        );
        let mut eng = self.eng.take();
        let mut async_mods = Vec::new();
        let mut frames = Vec::new();
        fetch_wave(&self.transport, self.max_inflight, jobs, |tag, res| {
            let Ok(resp) = res else { return };
            if !(200..300).contains(&resp.status) {
                return;
            }
            match tag {
                SubTag::Css(i) => {
                    self.css_slots[i] = Some(decode_stylesheet(&resp.body, &resp.headers));
                }
                SubTag::Script(slot) => {
                    let item = &self.scripts[slot_items[slot]];
                    // Étape 53b : charset déclaré
                    // (classique) → BOM → UTF-8.
                    let text = decode_subresource(&resp.body, item.charset.as_deref());
                    if item.exec == ScriptExec::Arrival {
                        if item.module {
                            if let Some(u) = &item.src {
                                async_mods.push((u.clone(), text));
                            }
                        } else {
                            eval_arrival(&mut eng, &text);
                        }
                    } else {
                        self.script_srcs[slot] = Some(text);
                    }
                }
                SubTag::Img(u) => {
                    self.img_bodies.insert(u.as_str(), resp.body);
                }
                SubTag::Frame(u) => {
                    frames.push(store_frame(&self.url, &self.cookies, &u, &resp));
                } // SubTag reste inchangé — `referrerpolicy` vit
                  // dans la `Request`, pas dans le routage.
            }
        })
        .await;
        self.eng = eng;
        self.async_mods = async_mods;
        self.frames = frames;
        self.fetch_imports().await;
        self.fetch_backgrounds().await;
        self.fetch_fonts().await;
        self.prefetch_modules().await;
    }

    /// Expansion des `@import` des feuilles liées (étape 96) :
    /// chaque feuille reçue est re-parsée, ses imports dont
    /// la media matche l'env viewport sont fetchés
    /// récursivement et **préfixés** au slot — le contenu
    /// importé prend la place de la règle `@import`, c.-à-d.
    /// *avant* les règles propres de la feuille. Bornes :
    /// `max_imports` feuilles total, `MAX_IMPORT_DEPTH` de
    /// profondeur, dédup par URL (`seen` neutralise les
    /// boucles). Avant `fetch_fonts` : un `@font-face`
    /// déclaré par une feuille importée est ainsi éligible.
    pub(crate) async fn fetch_imports(&mut self) {
        let mut ctx = ImportCtx {
            env: super::apply::viewport_env(self.viewport, self.color_scheme),
            seen: HashSet::new(),
            left: self.max_imports,
            out: String::new(),
        };
        for i in 0..self.css_slots.len() {
            if ctx.left == 0 {
                break;
            }
            let Some(text) = self.css_slots[i].take() else {
                continue;
            };
            let Some(base) = self.sheet_urls.get(i).cloned() else {
                self.css_slots[i] = Some(text);
                continue;
            };
            for imp in parse_stylesheet(&text).imports {
                self.expand_import(&imp, &base, 1, &mut ctx).await;
            }
            let prefix = std::mem::take(&mut ctx.out);
            self.css_slots[i] = Some(prefix + &text);
        }
    }

    /// Fetch + expansion récursive d'un `@import` — l'async
    /// récursif exige un future épinglé (`Box::pin`).
    /// Les imports *de l'importée* sont expansés avant son
    /// propre texte (ordre de cascade). Toute anomalie
    /// (media hors env, URL irrésolvable, schéma non-http,
    /// fetch/status non-2xx, borne atteinte) → import sauté,
    /// jamais d'échec propagé.
    fn expand_import<'a>(
        &'a mut self,
        imp: &'a ImportRule,
        base: &'a Url,
        depth: usize,
        ctx: &'a mut ImportCtx,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            if depth > MAX_IMPORT_DEPTH
                || ctx.left == 0
                || !media_attr_matches(&ctx.env, &imp.media)
            {
                return;
            }
            let Ok(url) = Url::resolve(base, &imp.url) else {
                return;
            };
            if !matches!(url.scheme(), "http" | "https") || !ctx.seen.insert(url.to_string()) {
                return;
            }
            ctx.left -= 1;
            let Ok(resp) = self
                .transport
                .fetch(&sub_request(
                    &self.url,
                    &url,
                    FetchDest::Style,
                    self.page_policy,
                ))
                .await
            else {
                return;
            };
            if !(200..300).contains(&resp.status) {
                return;
            }
            let text = decode_stylesheet(&resp.body, &resp.headers);
            for sub in parse_stylesheet(&text).imports {
                self.expand_import(&sub, &url, depth + 1, ctx).await;
            }
            ctx.out.push_str(&text);
            ctx.out.push('\n');
        })
    }

    /// Fetch des `url(http…)` d'arrière-plan **et de masque**
    /// (étapes 98 et 121 — `mask-image`/`-webkit-mask-image`,
    /// demande PIPE R-86) : les déclarations des feuilles
    /// liées (imports expansés compris — leur texte a rejoint
    /// le slot de la parente et se résout contre sa base) et
    /// du `<style>` inline sont re-parcourues ; chaque
    /// `url(…)` est résolu sur **sa** feuille, déduplé par
    /// URL résolue et fetché dans le budget `max_images`
    /// restant après `<img>`. `bg_aliases`/`mask_aliases`
    /// conservent le lien texte-brut → résolue que la valeur
    /// calculée a perdu (la feuille d'origine n'y survit
    /// pas). Séquentiel — toute anomalie (URL irrésolvable,
    /// schéma non-http, fetch/status en échec, budget
    /// épuisé) → ressource sautée, jamais d'échec propagé.
    /// Fonds avant masques dans la même boucle : un même
    /// corps `bg_bodies` sert les deux rôles.
    pub(crate) async fn fetch_backgrounds(&mut self) {
        const BG_PROPS: &[&str] = &["background", "background-image"];
        const MASK_PROPS: &[&str] = &["mask-image", "-webkit-mask-image"];
        let env = super::apply::viewport_env(self.viewport, self.color_scheme);
        let mut left = self.max_images.saturating_sub(self.img_urls.len());
        let mut seen = HashSet::new();
        let mut decl_urls = Vec::new();
        for (i, slot) in self.css_slots.iter().enumerate() {
            let Some(text) = slot else { continue };
            let base = self
                .sheet_urls
                .get(i)
                .cloned()
                .unwrap_or_else(|| self.url.clone());
            let rules = parse_stylesheet(text).rules;
            let mut bg = Vec::new();
            collect_decl_urls(&rules, &env, BG_PROPS, &mut bg);
            let mut mask = Vec::new();
            collect_decl_urls(&rules, &env, MASK_PROPS, &mut mask);
            decl_urls.push((base, bg, mask));
        }
        {
            let mut bg = Vec::new();
            let mut mask = Vec::new();
            for sheet in &self.doc_sheets {
                let rules = parse_stylesheet(sheet).rules;
                collect_decl_urls(&rules, &env, BG_PROPS, &mut bg);
                collect_decl_urls(&rules, &env, MASK_PROPS, &mut mask);
            }
            decl_urls.push((self.url.clone(), bg, mask));
        }
        'slots: for (base, bg_urls, mask_urls) in decl_urls {
            for (raw, is_mask) in bg_urls
                .into_iter()
                .map(|r| (r, false))
                .chain(mask_urls.into_iter().map(|r| (r, true)))
            {
                if left == 0 {
                    break 'slots;
                }
                if raw.starts_with("data:") {
                    continue;
                }
                let Ok(url) = Url::resolve(&base, &raw) else {
                    continue;
                };
                if !matches!(url.scheme(), "http" | "https") || !seen.insert(url.to_string()) {
                    continue;
                }
                left -= 1;
                let Ok(resp) = self
                    .transport
                    .fetch(&sub_request(
                        &self.url,
                        &url,
                        FetchDest::Image,
                        self.page_policy,
                    ))
                    .await
                else {
                    continue;
                };
                if !(200..300).contains(&resp.status) {
                    continue;
                }
                if is_mask {
                    self.mask_aliases.insert(raw, url.to_string());
                } else {
                    self.bg_aliases.insert(raw, url.to_string());
                }
                self.bg_bodies.insert(url.to_string(), resp.body);
            }
        }
    }

    /// Fetch + décode des `@font-face` hors session (étape
    /// 53d — ex-`fetch_webfonts` déplacé) : faces résolues
    /// sur `doc_sheets` + feuilles liées téléchargées, chaque
    /// source `url()` essayée dans l'ordre (première qui
    /// décode gagne) ; `register` au commit borne le
    /// registre. Séquentiel — `font_budget` vus.
    pub(crate) async fn fetch_fonts(&mut self) {
        let mut eng = StyleEngine::new();
        // Une passe par feuille (étape 104) — même ordre
        // document que la cascade appliquée.
        for sheet in &self.doc_sheets {
            eng.add_css(sheet, Origin::Author);
        }
        for sheet in self.css_slots.iter().flatten() {
            eng.add_css(sheet, Origin::Author);
        }
        let env = super::apply::viewport_env(self.viewport, self.color_scheme);
        'faces: for face in eng.font_faces(&env) {
            if self.fonts.len() >= self.font_budget {
                break;
            }
            for src in &face.srcs {
                let FontSrc::Url(href) = src else {
                    continue;
                };
                let Ok(url) = Url::resolve(&self.url, href) else {
                    continue;
                };
                let mut req = Request::get(url);
                req.initiator = Some(Initiator::Subresource(self.url.clone()));
                req.fetch_dest = FetchDest::Font;
                req.referrer_policy = self.page_policy;
                req.headers.extend(initiator_headers_policy(
                    &self.url,
                    &req.url,
                    Method::Get,
                    self.page_policy.unwrap_or_default(),
                ));
                req.headers.extend(sec_fetch_headers(&req));
                let Ok(resp) = self.transport.fetch(&req).await else {
                    continue;
                };
                if !(200..300).contains(&resp.status) {
                    continue;
                }
                let Some(data) = decode_font(&resp.body) else {
                    continue;
                };
                self.fonts
                    .push((face.family.clone(), face.weight, face.italic, data));
                continue 'faces;
            }
        }
    }

    /// Prefetch des graphes de modules hors session (étape
    /// 53d — ex-`eval_module_src` splitté) : même ordre que
    /// la file d'exécution du commit — `async` en ordre
    /// d'arrivée puis post-parse en ordre document.
    pub(crate) async fn prefetch_modules(&mut self) {
        let mut entries: Vec<(String, Url, String)> = Vec::new();
        for (u, src) in std::mem::take(&mut self.async_mods) {
            entries.push((u.as_str(), u, src));
        }
        let async_n = entries.len();
        for (i, item) in self.scripts.iter().enumerate() {
            if item.exec != ScriptExec::Post || !item.module {
                continue;
            }
            let Some(src) = item_source(item, &self.script_srcs) else {
                continue;
            };
            // Entrée inline : nom synthétique unique, aliasé
            // vers la page pour la résolution des deps.
            let (name, base_u) = item.src.clone().map_or_else(
                || (format!("inline:{i}"), self.url.clone()),
                |u| (u.as_str(), u),
            );
            entries.push((name, base_u, src.to_string()));
        }
        let mut post_evals = Vec::new();
        for (n, (name, base, src)) in entries.into_iter().enumerate() {
            let mut graph = ModuleGraph {
                sources: HashMap::from([(name.clone(), src.clone())]),
                alias: HashMap::from([(name.clone(), base.as_str())]),
            };
            Session::fetch_module_sources(
                &self.transport,
                self.max_inflight,
                self.max_modules,
                &self.page,
                self.page_policy,
                &base,
                &src,
                &self.import_specs,
                &mut graph,
            )
            .await;
            let me = ModEval { name, graph };
            if n < async_n {
                self.arrival_evals.push(me);
            } else {
                post_evals.push(me);
            }
        }
        self.post_evals = post_evals;
    }
}

/// Complétion d'une requête script-initiée (étape 51) —
/// produite par la tâche tokio spawned, consommée par
/// [`Session::pump_net`] aux points de synchro.
pub(crate) struct NetDone {
    /// Onglet émetteur.
    pub(crate) tab: TabId,
    /// Corrélation côté moteur (`NetResponse::handle`).
    pub(crate) handle: u64,
    /// Requête cross-origin — la lisibilité de la réponse
    /// est conditionnée à `Access-Control-Allow-Origin`
    /// (enforcement `CorsTransport`, étape 126).
    pub(crate) cross: bool,
    /// `credentials:` de la requête — `Set-Cookie` capturé
    /// same-origin (sauf `omit`) ou `include` (étape 126).
    pub(crate) credentials: explorer_net::http::FetchCredentials,
    /// Origine canonique de la page émettrice (check ACAO).
    pub(crate) page_origin: String,
    /// URL de la requête — capture `Set-Cookie`.
    pub(crate) req_url: Url,
    /// Seau CHIPS de la requête (étape 127) — un `Set-Cookie:
    /// …; Partitioned` se range sous cette clé.
    pub(crate) partition: Option<String>,
    /// Sortie brute du transport.
    pub(crate) out: NetResult<Response>,
}

/// Verbe d'un `NetRequest` → `Method` — un verbe hors
/// sous-ensemble usuel n'est pas émis (dégradation status 0).
pub(crate) fn parse_method(s: &str) -> Option<Method> {
    Some(match s.trim().to_ascii_uppercase().as_str() {
        "GET" => Method::Get,
        "HEAD" => Method::Head,
        "POST" => Method::Post,
        "PUT" => Method::Put,
        "DELETE" => Method::Delete,
        "OPTIONS" => Method::Options,
        _ => return None,
    })
}

/// En-tête safelisté CORS (nom) — un en-tête auteur hors
/// liste figure dans `CorsRequest::unsafe_headers` et
/// déclenche le preflight `OPTIONS` du `CorsTransport`
/// (étape 126 ; la vérification `Access-Control-Allow-
/// Headers` est côté transport).
pub(crate) fn cors_safelisted_header(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_lowercase().as_str(),
        "accept" | "accept-language" | "content-language" | "content-type" | "range"
    )
}

/// En-têtes interdits au script (Fetch « forbidden header
/// names », sous-ensemble) — le navigateur garde le contrôle
/// de `Referer`/`Origin`/`Cookie`/`Host`…, jamais posés par
/// la page (filtrés silencieusement à l'émission).
pub(crate) fn forbidden_request_header(name: &str) -> bool {
    let n = name.trim().to_ascii_lowercase();
    n.starts_with("sec-")
        || n.starts_with("proxy-")
        || matches!(
            n.as_str(),
            "accept-charset"
                | "accept-encoding"
                | "access-control-request-headers"
                | "access-control-request-method"
                | "connection"
                | "content-length"
                | "cookie"
                | "date"
                | "expect"
                | "host"
                | "keep-alive"
                | "origin"
                | "referer"
                | "te"
                | "trailer"
                | "transfer-encoding"
                | "upgrade"
                | "via"
        )
}

/// Réponse cross-origin lisible par le script ? CORS minimal :
/// `Access-Control-Allow-Origin: *` ou égal à l'origine
/// canonique de la page — sinon la réponse est opaque
/// (`status: 0` → événement `error`, comme un échec réseau).
pub(crate) fn cors_readable(page_origin: &str, resp: &Response) -> bool {
    resp.headers_all("access-control-allow-origin").any(|v| {
        let v = v.trim();
        v == "*" || v == page_origin
    })
}

/// `Origin` canonique d'une page — `scheme://host[:port]`,
/// port omis s'il est le défaut du schéma ; origine opaque
/// sérialisée `null` (spec Fetch, valeur `Origin` en-tête).
pub(crate) fn origin_header_value(u: &Url) -> String {
    let default = match u.scheme() {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    };
    match u.effective_port() {
        Some(p) if Some(p) != default => {
            format!("{}://{}:{}", u.scheme(), u.host(), p)
        }
        Some(_) => format!("{}://{}", u.scheme(), u.host()),
        None => "null".to_string(),
    }
}

/// `document.cookie` ne peut pas poser `HttpOnly` — l'attribut
/// est ignoré (spec : le cookie est posé, visible côté script).
pub(crate) fn strip_http_only(raw: &str) -> String {
    raw.split(';')
        .enumerate()
        .filter(|(i, attr)| *i == 0 || !attr.trim().eq_ignore_ascii_case("httponly"))
        .map(|(_, s)| s.trim())
        .collect::<Vec<_>>()
        .join("; ")
}

/// File de jobs de sous-ressources, émise par priorité
/// `CSS > script > img` — le scheduler `fetch_wave` les tire
/// dans cet ordre (étape 50 ; le document est déjà servi).
pub(crate) fn sub_jobs(
    base: &Url,
    sheet_urls: &[Url],
    scripts: &[ScriptItem],
    img_urls: &[Url],
    frame_urls: &[(Url, Option<ReferrerPolicy>)],
    page_policy: Option<ReferrerPolicy>,
) -> VecDeque<(SubTag, Request)> {
    let mut jobs = VecDeque::new();
    for (i, u) in sheet_urls.iter().enumerate() {
        jobs.push_back((
            SubTag::Css(i),
            sub_request(base, u, FetchDest::Style, page_policy),
        ));
    }
    for item in scripts {
        if let (Some(u), Some(slot)) = (&item.src, item.slot) {
            jobs.push_back((
                SubTag::Script(slot),
                sub_request(base, u, FetchDest::Script, page_policy),
            ));
        }
    }
    for u in img_urls {
        jobs.push_back((
            SubTag::Img(u.clone()),
            sub_request(base, u, FetchDest::Image, page_policy),
        ));
    }
    for (u, policy) in frame_urls {
        jobs.push_back((
            SubTag::Frame(u.clone()),
            frame_request(base, u, policy.or(page_policy)),
        ));
    }
    jobs
}

/// Requête GET de sous-ressource : `Referer`/`Origin` calculés
/// à l'émission (`initiator_headers_policy` sous la
/// `policy` effective — attribut élément ou politique du
/// document, étape 132) + métadonnées `Sec-Fetch-*` (étape
/// 125 — `dest` posé par l'appelant, `mode` = `no-cors`
/// classique) — le mixed-content est filtré en amont.
pub(crate) fn sub_request(
    base: &Url,
    url: &Url,
    dest: FetchDest,
    policy: Option<ReferrerPolicy>,
) -> Request {
    let mut req = Request::get(url.clone());
    req.initiator = Some(Initiator::Subresource(base.clone()));
    req.fetch_dest = dest;
    req.referrer_policy = policy;
    for (n, v) in initiator_headers_policy(base, url, Method::Get, policy.unwrap_or_default()) {
        req.headers.push((n, v));
    }
    req.headers.extend(sec_fetch_headers(&req));
    req
}

/// Requête de navigation d'un sous-contexte `<iframe>`
/// (étape 128) : même canal `sub_request` (initiateur =
/// page parente → `SameSite`/`CHIPS` corrects — un cookie
/// `Partitioned` du seau parent est admissible, un
/// `Lax` cross-site ne l'est pas, spec cookies) mais mode
/// `navigate` à l'émission — `sec-fetch-dest: iframe` +
/// `sec-fetch-mode: navigate` comme une vraie navigation
/// imbriquée (spec Fetch metadata). `policy` = politique
/// effective (attribut `referrerpolicy=` de l'élément ou
/// celle du document — étape 132).
pub(crate) fn frame_request(base: &Url, url: &Url, policy: Option<ReferrerPolicy>) -> Request {
    let mut req = sub_request(base, url, FetchDest::Iframe, policy);
    req.fetch_mode = FetchMode::Navigate;
    // `sec-fetch-*` recalculés après le changement de mode.
    req.headers.retain(|(n, _)| !n.starts_with("sec-fetch-"));
    req.headers.extend(sec_fetch_headers(&req));
    req
}

/// Réception d'un document `<iframe>` (étape 128) :
/// `Set-Cookie` capturé dans le jar partagé — seau CHIPS =
/// site top-level de la **page parente** (profondeur 1 :
/// le parent EST le top-level, même calcul que
/// `CookieTransport` côté pile daemon) — puis le corps
/// HTML est décodé (charset transport en amorce, comme la
/// nav) et parsé en `Dom` propre : aucune mutation croisée
/// avec le parent n'existe.
pub(crate) fn store_frame(page: &Url, cookies: &CookieJar, u: &Url, resp: &Response) -> SubFrame {
    let partition = partition_key(page);
    for v in resp.headers_all("set-cookie") {
        let _ = cookies.set_from_header_ctx(u, v, partition.clone());
    }
    let text = decode_subresource(&resp.body, transport_charset(&resp.headers).as_deref());
    let dom = TreeBuilder::parse(&text);
    let title = extract_title(&dom);
    SubFrame {
        url: u.clone(),
        dom,
        title,
    }
}

/// Profondeur max d'une chaîne `@import` (étape 96) —
/// complément de la dédup par URL : borne la descente sur
/// graphes dégénérés longs-sans-cycle.
pub(crate) const MAX_IMPORT_DEPTH: usize = 8;

/// Contexte d'expansion `@import` partagé par la descente
/// récursive : env d'évaluation media, URLs déjà vues
/// (dédup = anti-boucle), budget restant, tampon de
/// préfixe collecté.
struct ImportCtx {
    env: StyleEnv,
    seen: HashSet<String>,
    left: usize,
    out: String,
}

/// Évalue un prélude media brut (`media=` de `<link>`,
/// prélude d'`@import`) contre l'env de session. Shim : le
/// texte est reparsé comme prélude d'une règle `@media`
/// synthétique — réutilise le parseur `explorer-css`, aucune
/// API nouvelle côté PIPE. Vide ou inclassable → `true` :
/// on ne masque jamais une feuille par doute (tolérance).
pub(crate) fn media_attr_matches(env: &StyleEnv, media: &str) -> bool {
    let media = media.trim();
    if media.is_empty() {
        return true;
    }
    let shim = parse_stylesheet(&format!("@media {media} {{a{{b:c}}}}"));
    let queries = shim.rules.iter().find_map(|r| match r {
        Rule::At(AtRule {
            name,
            kind: AtKind::Media { queries, .. },
        }) if name == "media" => Some(queries),
        _ => None,
    });
    queries.is_none_or(|q| media_matches(env, q))
}

/// Vague de fetches parallèle bornée (étape 50, scheduler
/// ADR-0012) : `jobs` émis dans l'ordre fourni (priorité =
/// position), `max_inflight` requêtes en vol via
/// `FuturesUnordered` ; `on_done(tag, résultat)` est appelé
/// dans l'ordre d'arrivée, en ligne, entre deux émissions.
pub(crate) async fn fetch_wave<T>(
    transport: &Arc<dyn HttpTransport>,
    max_inflight: usize,
    mut jobs: VecDeque<(T, Request)>,
    mut on_done: impl FnMut(T, NetResult<Response>),
) {
    let cap = max_inflight.max(1);
    let mut inflight = FuturesUnordered::new();
    loop {
        while inflight.len() < cap {
            let Some((tag, req)) = jobs.pop_front() else {
                break;
            };
            let tr = Arc::clone(transport);
            inflight.push(async move {
                let res = tr.fetch(&req).await;
                (tag, res)
            });
        }
        if inflight.is_empty() {
            return;
        }
        let Some((tag, res)) = inflight.next().await else {
            return;
        };
        on_done(tag, res);
    }
}

/// Spécificateurs statiques `(spec brut, url résolue)` d'un
/// corps de module — `module_specifiers` (explorer-jscore,
/// JS-C5) + réécriture des bare via l'import map de la page
/// (issue #6 — `rewrite_bare_specifier`, même règle que le
/// moteur à l'eval) puis résolution contre l'URL du module
/// parent. `map` = `imports` fusionné (`merge_import_maps`).
/// Le `spec` rendu reste la forme **brute** : jscore
/// enregistre chaque module sous son spécificateur d'origine,
/// c'est donc la clé qui servira de `referrer` à l'eval —
/// `alias` doit la traduire vers l'URL résolue.
pub(crate) fn module_deps(
    src: &str,
    referrer: &Url,
    map: &[(String, String)],
) -> Vec<(String, Url)> {
    explorer_js::module_specifiers(src)
        .into_iter()
        .filter_map(|spec| {
            let resolved =
                explorer_js::rewrite_bare_specifier(map, &spec).unwrap_or_else(|| spec.clone());
            // Bare resté hors map : « unresolvable » par spec —
            // ne pas le dégénérer en chemin relatif.
            if explorer_js::is_bare_specifier(&resolved) {
                return None;
            }
            Url::resolve(referrer, &resolved).ok().map(|u| (spec, u))
        })
        .collect()
}

/// `url(…)` des déclarations listées (`props`) d'un jeu de
/// règles, `@media` imbriqués évalués contre `env` (étape
/// 98, généralisé étape 121 pour `mask-image`) — une
/// feuille hors-media ne fetch rien. `@keyframes` ignorés :
/// un `url()` dedans serait animé, hors périmètre raster
/// mono-image.
fn collect_decl_urls(rules: &[Rule], env: &StyleEnv, props: &[&str], out: &mut Vec<String>) {
    for r in rules {
        match r {
            Rule::Qualified(q) => {
                for d in &q.declarations {
                    if props.contains(&d.name.as_str()) {
                        css_url_args(&d.value, out);
                    }
                }
            }
            Rule::At(a) => {
                if let AtKind::Media { queries, rules } = &a.kind {
                    if media_matches(env, queries) {
                        collect_decl_urls(rules, env, props, out);
                    }
                }
            }
        }
    }
}
