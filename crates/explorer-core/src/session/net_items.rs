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
    FetchCredentials, FetchDest, FetchMode, HttpTransport, Initiator, Method, ReferrerPolicy,
    Request, Response, StreamedResponse,
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
    /// Feuilles `<link rel=stylesheet>` résolues —
    /// `(url, integrity, crossorigin)` : SRI (CAP-163) et
    /// mode CORS (CAP-167) voyagent jusqu'à l'émission et
    /// au commit de la réponse.
    pub(crate) sheets: Vec<SheetFetch>,
    /// Scripts collectés (`<script src>` + inline/modules).
    pub(crate) scripts: Vec<ScriptItem>,
    /// `<img>`/`poster` résolus et dédupliqués.
    pub(crate) imgs: Vec<Url>,
    /// `<iframe src>` résolus — navigations de
    /// sous-contexte (bornés `max_frames`, partagé avec
    /// `srcdoc`) ; la `referrerpolicy` de l'élément voyage
    /// avec l'URL (étape 132) ainsi que ses `sandbox`
    /// (CAP-149, `None` = plein droit).
    pub(crate) frames: Vec<(Url, Option<ReferrerPolicy>, Option<SandboxFlags>)>,
    /// `<iframe srcdoc>` — documents **inline** (CAP-142) :
    /// aucun fetch, le HTML de l'attribut est parsé tel
    /// quel en `SubFrame` (`about:srcdoc`) à la vague ; les
    /// `sandbox` de l'élément suivent (CAP-149).
    pub(crate) srcdoc_frames: Vec<(String, Option<SandboxFlags>)>,
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
    /// parsé en `Dom` propre au commit. Index dans
    /// `SubJob::frame_urls` (CAP-149 : l'URL, la
    /// `referrerpolicy` ET les `sandbox` voyagent ensemble).
    Frame(usize),
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
    /// « scripting flag » pour le parse (CAP-155 — spec
    /// §13.2.3.5 : `<noscript>` = rawtext quand le JS
    /// tournera). Cloné depuis `js_factory.is_some()` au
    /// `nav_begin` — le job détaché n'a pas la session.
    pub(crate) scripting: bool,
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
    /// `<base href>` résolu contre l'URL du document
    /// (étape 135) — `None` = la base de résolution reste
    /// l'URL du document.
    pub(crate) base_url: Option<Url>,
    /// `<base target>` brut (étape 135) — cible par défaut
    /// des `<a>`/`<form>` sans `target` propre.
    pub(crate) base_target: Option<String>,
    /// `<meta http-equiv=refresh>` (CAP-143) — `(délai,
    /// cible résolue sur la base)` ; `None` = pas de
    /// navigation programmée. L'échéance est posée au
    /// commit (`Instant` recalé sur l'apply).
    pub(crate) refresh: Option<(std::time::Duration, Url)>,
    /// Politiques CSP du document (CAP-145) — en-têtes
    /// `Content-Security-Policy:` de la réponse **plus**
    /// `<meta http-equiv=Content-Security-Policy>` en
    /// ordre document (chacune doit autoriser — la plus
    /// restrictive réduit ; `Report-Only` ignoré).
    pub(crate) csp: Vec<explorer_net::csp::CspPolicy>,
    /// `Clear-Site-Data: "cookies"` ou `"*"` reçu sur la
    /// réponse (CAP-147) — le jar du site est vidé au
    /// commit (après la navigation, comme la spec : la
    /// page servie tourne déjà sans identifiants).
    pub(crate) clear_cookies: bool,
}

/// Issue de [`NavJob::run`] (étape 70) : document à commettre
/// ou téléchargement bufferisé à proposer.
pub(crate) enum NavOutcome {
    /// Document affichable — commit `nav_apply` habituel.
    /// Boxé : `NavDone` (DOM + métadonnées) écrase la
    /// différence de taille avec `Download` (étape 135).
    Document(Box<NavDone>, Box<NavTail>),
    /// Corps non affichable bufferisé — la session le garde
    /// en attente d'acceptation (`PendingDownload`, queue de
    /// navigation incluse pour le rejeu « dump document »
    /// d'un rejet).
    Download(Box<super::downloads::PendingDownload>),
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
    // Une ressource **script** servie en top-level n'est
    // pas un document — `text/javascript` serait sinon
    // ratatché à `text/*` et peint en brut (vol rakuten :
    // `window.open` vers un `.js` ouvrait un onglet
    // affichant la source). Politique navigateurs : ces
    // types basculent en téléchargement.
    let script_like = matches!(
        mime.as_str(),
        "application/javascript"
            | "application/x-javascript"
            | "application/ecmascript"
            | "application/x-ecmascript"
            | "text/javascript"
            | "text/ecmascript"
            | "text/jscript"
            | "text/livescript"
            | "text/x-javascript"
            | "text/x-ecmascript"
    );
    let doc_like = !script_like
        && (mime.is_empty()
            || mime == "text/html"
            || mime == "application/xhtml+xml"
            || mime.starts_with("text/")
            || mime.ends_with("+xml"));
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

/// `true` si la réponse top-level est un **document image**
/// (politique navigateurs — l'« image document ») :
/// `Content-Type: image/*`, pas de `Content-Disposition:
/// attachment`, pas de `<a download>` (CAP-148 — le hint
/// force le download même sur type affichable). Rendue en
/// page synthétique `<img>` plutôt qu'en téléchargement —
/// un clic de lien-image navigue alors comme attendu
/// (écart terrain stallman mesuré : `lien:clic-sans-nav`
/// était une offre de download invisible).
fn image_document_eligible(headers: &[(String, String)], req: &Request) -> bool {
    if req.download_hint.is_some() {
        return false;
    }
    let get = |name: &str| {
        headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    };
    let attached = get("content-disposition")
        .unwrap_or_default()
        .split(';')
        .next()
        .is_some_and(|d| d.trim().eq_ignore_ascii_case("attachment"));
    let mime = get("content-type")
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    !attached && mime.starts_with("image/")
}

/// Échappement minimal pour un nom inséré dans du HTML
/// synthétique (attribut `alt` + `<title>`) — l'URL elle-
/// même est déjà normalisée par le type `Url`.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Méta de téléchargement de la réponse : en-têtes
/// d'abord (étape 70), puis `<a download>` forcé
/// (CAP-148 — le hint est posé au dispatch lien, la garde
/// same-origin de la spec y est vérifiée).
fn response_download_meta(headers: &[(String, String)], req: &Request) -> Option<DownloadMeta> {
    download_meta(headers).or_else(|| {
        req.download_hint
            .as_deref()
            .map(|attr| forced_download_meta(headers, attr))
    })
}

/// Meta d'un téléchargement **forcé** par `<a download>`
/// (CAP-148) — la voie s'impose même quand le type est
/// affichable (HTML « downloading resources »). Priorité
/// du nom : `filename=` du `Content-Disposition`, puis la
/// valeur non vide de l'attribut, puis (en aval) le
/// segment d'URL — même ordre que la spec.
fn forced_download_meta(headers: &[(String, String)], attr: &str) -> DownloadMeta {
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
    let attr = attr.trim();
    DownloadMeta {
        filename_hint: disposition_filename(disposition)
            .or_else(|| (!attr.is_empty()).then(|| attr.to_string())),
        mime: if mime.is_empty() {
            "application/octet-stream".to_string()
        } else {
            mime
        },
        total,
    }
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

/// CSP du document (CAP-145) : **toutes** les en-têtes
/// `Content-Security-Policy` de la réponse (chaque
/// politique s'applique — la plus restrictive réduit),
/// puis les `<meta http-equiv>` du DOM (`via_meta` :
/// `frame-ancestors`/`sandbox`/`report-*` y sont écartées
/// par spec au parse).
fn collect_response_csp(
    headers: &[(String, String)],
    dom: &Dom,
) -> Vec<explorer_net::csp::CspPolicy> {
    let mut csp: Vec<explorer_net::csp::CspPolicy> = headers
        .iter()
        .filter(|(n, _)| n.eq_ignore_ascii_case("content-security-policy"))
        .map(|(_, v)| explorer_net::csp::CspPolicy::parse(v, false))
        .collect();
    csp.extend(
        collect_meta_csp(dom)
            .iter()
            .map(|v| explorer_net::csp::CspPolicy::parse(v, true)),
    );
    csp
}

/// `Clear-Site-Data` (CAP-147) — tokens quotés ; `"cookies"`
/// et `"*"` demandent le vidage du jar du site au commit.
/// `"cache"`/`"storage"`/`"executionContexts"` non
/// implémentés (cache disque côté hôte, storage JS absent)
/// — bornes documentées.
fn wants_clear_cookies(headers: &[(String, String)]) -> bool {
    headers.iter().any(|(n, v)| {
        n.eq_ignore_ascii_case("clear-site-data")
            && (v.contains("\"cookies\"") || v.contains("\"*\""))
    })
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
    #[allow(clippy::too_many_lines)] // flux séquentiel — chaque phase est documentée in situ
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
        // `Refresh:` de la réponse (CAP-144) — pragma HTTP,
        // traitée **avant** les `<meta>` du document (spec :
        // l'en-tête prime) ; première valeur valide gagne.
        let header_refresh = resp
            .headers
            .iter()
            .filter(|(n, _)| n.eq_ignore_ascii_case("refresh"))
            .find_map(|(_, v)| parse_refresh_content(v));
        // Document image (`image/*` non attaché, pas de
        // `<a download>`) : page synthétique `<img>` rendue
        // comme les navigateurs — avant la classification
        // download qui l'enfermait en offre invisible.
        if image_document_eligible(&resp.headers, &self.req) {
            return self.buffer_image_document(resp, resolved).await;
        }
        // Corps non affichable (étape 70) ou `<a download>`
        // (CAP-148) : bufferisé avec borne et progression —
        // jamais décodé en mojibake.
        if let Some(meta) = response_download_meta(&resp.headers, &self.req) {
            return self.buffer_download(resp, resolved, meta).await;
        }
        // `<noscript>` = rawtext si le JS tournera (CAP-155).
        let mut tb = scripted_tree_builder(self.scripting);
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
        let csp = collect_response_csp(&resp.headers, &dom);
        let clear_cookies = wants_clear_cookies(&resp.headers);
        // `<base>` résolu contre l'URL de la réponse (étape
        // 135) — la résolution des sous-ressources, liens et
        // formulaires suit la base déclarée. `base-uri`
        // (CAP-145) : une base refusée est ignorée.
        let (base_url, base_target) = document_base(&dom, &resolved);
        let base_url =
            base_url.filter(|b| explorer_net::csp::csp_base_uri_allowed(&csp, &resolved, b));
        // Refresh (CAP-143 + CAP-144) : l'en-tête `Refresh:`
        // prime sur la `<meta>` du document — cible résolue
        // sur la base effective ; sans `url` c'est un
        // rechargement (la cible est l'URL du document).
        let refresh_base = base_url.clone().unwrap_or_else(|| resolved.clone());
        let refresh = header_refresh
            .or_else(|| meta_refresh(&dom))
            // Repli `noscript` (vol google) : page de
            // fallback pure — scripting levé au parse, la
            // méta du repli est honorée.
            .or_else(|| noscript_meta_refresh(&dom))
            .and_then(|(secs, raw)| {
                let target = match raw {
                    Some(r) => Url::resolve(&refresh_base, &r).ok()?,
                    None => resolved.clone(),
                };
                std::time::Duration::try_from_secs_f64(secs.max(0.0))
                    .ok()
                    .map(|d| (d, target))
            });
        let done = NavDone {
            url: resolved,
            // `<meta name=referrer>` l'emporte sur
            // l'en-tête (traitée après lui, spec).
            referrer_policy: meta_referrer_policy(&dom).or(header_policy),
            base_url,
            base_target,
            refresh,
            csp,
            clear_cookies,
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
        Ok(NavOutcome::Document(Box::new(done), Box::new(tail)))
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
        Ok(NavOutcome::Download(Box::new(
            super::downloads::PendingDownload {
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
            },
        )))
    }

    /// Document image top-level (« image document » des
    /// navigateurs) : le corps est bufferisé avec la même
    /// borne qu'un download — un blob hostile géant se
    /// borne pareil — puis une page synthétique
    /// `<img src="url">` est commise : le pipeline `<img>`
    /// existant re-fetch/décode/rend (un mime `image/*`
    /// indécodable retombe sur le texte `alt`, dégradation
    /// maison). Titre = segment final de l'URL, comme les
    /// navigateurs.
    async fn buffer_image_document(
        &mut self,
        mut resp: StreamedResponse,
        resolved: Url,
    ) -> Result<NavOutcome, CoreError> {
        let mut n = 0usize;
        while let Some(chunk) = resp.body.recv().await {
            let bytes = chunk.map_err(|e| CoreError::Fetch(e.to_string()))?;
            n += bytes.len();
            if n > self.max_download_bytes {
                return Err(CoreError::Fetch("image hors bornes".to_string()));
            }
        }
        let name = resolved
            .path()
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("image");
        let name = html_escape(name);
        let html = format!(
            "<!doctype html><html><head><title>{name}</title></head>\
             <body style=\"margin:0\"><img src=\"{resolved}\" alt=\"{name}\"></body></html>"
        );
        let mut tb = scripted_tree_builder(self.scripting);
        tb.push(&html);
        tb.finish();
        let dom = tb.into_dom();
        let title = extract_title(&dom);
        let csp = collect_response_csp(&resp.headers, &dom);
        let done = NavDone {
            url: resolved,
            dom,
            html,
            title,
            referrer_policy: None,
            base_url: None,
            base_target: None,
            refresh: None,
            csp,
            clear_cookies: wants_clear_cookies(&resp.headers),
        };
        let tail = NavTail {
            epoch: self.epoch,
            push: self.push,
            prev_title: self.prev_title.take(),
            req: self.req.clone(),
        };
        Ok(NavOutcome::Document(Box::new(done), Box::new(tail)))
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
            Box::new(NavDone {
                url: self.req.url.clone(),
                dom: tb.into_dom(),
                html: String::new(),
                title: None,
                referrer_policy: None,
                base_url: None,
                base_target: None,
                refresh: None,
                csp: Vec::new(),
                clear_cookies: false,
            }),
            Box::new(tail),
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
    /// Base de résolution des URLs relatives du document
    /// (étape 135) — `<base href>` résolu, `url` sinon.
    /// `self.url` reste la page (initiateur, `Referer`,
    /// mixed-content) ; `doc_base` ne sert qu'à résoudre.
    pub(crate) doc_base: Url,
    /// Plan de collecte — URLs résolues à `nav_apply` ;
    /// `(url, integrity, crossorigin)` (CAP-163/167).
    pub(crate) sheet_urls: Vec<SheetFetch>,
    pub(crate) scripts: Vec<ScriptItem>,
    pub(crate) img_urls: Vec<Url>,
    /// `<iframe src>` résolus à `nav_apply` (étape 128 —
    /// déjà dédupliqués/mixed-content-filtrés/bornés) ;
    /// la `referrerpolicy` de l'élément voyage avec l'URL
    /// (étape 132) ainsi que ses `sandbox` (CAP-149).
    pub(crate) frame_urls: Vec<(Url, Option<ReferrerPolicy>, Option<SandboxFlags>)>,
    /// `<iframe srcdoc>` — documents inline (CAP-142) :
    /// pas de requête, le HTML est parsé en `SubFrame`
    /// (`about:srcdoc`) après la vague — la borne
    /// `max_frames` est déjà appliquée en plan ; les
    /// `sandbox` de l'élément suivent (CAP-149).
    pub(crate) srcdoc_frames: Vec<(String, Option<SandboxFlags>)>,
    /// Politiques CSP du document (CAP-145) — servent les
    /// fetchs tardifs (`@import`, fonds/masques, fonts,
    /// modules) : le plan a déjà éliminé les sous-
    /// ressources interdites, ici les émissions suivent
    /// la même règle.
    pub(crate) csp: Vec<explorer_net::csp::CspPolicy>,
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
    /// « scripting flag » de la page (R-93) — dupliqué hors
    /// de `eng` : le moteur est `take()`n en cours de vague
    /// alors que l'env des feuilles (`@media (scripting)`)
    /// doit rester lisible jusqu'aux imports/fontes.
    pub(crate) scripting: bool,
    /// Réceptions de la vague — indexées par ordre document.
    pub(crate) css_slots: Vec<Option<String>>,
    /// `(el_ord, ok)` des `<link rel=stylesheet>` statiques
    /// (CAP-185) — enregistrés à la réception (fetch, 2xx,
    /// sécurité sous-ressource, SRI) ; la session livre
    /// `load`/`error` sur l'élément miroir au commit, une
    /// fois les scripts du document évalués (les listeners
    /// `addEventListener(..., true)` doivent exister — le
    /// bootstrap `x-web` n'injecte son entrée que là-dessus).
    pub(crate) link_events: Vec<(u32, bool)>,
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
    #[allow(clippy::too_many_lines)] // dispatch séquentiel — chaque branche est documentée in situ
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
        let eval_arrival = |eng: &mut Option<Box<dyn JsEngine>>, src: &str, ord: usize| {
            eval_async_arrival(eng, src, ord, tab, &events);
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
        // Scripting du parent (CAP-155) — hissé hors de la
        // closure `!Sync`-sûre (le moteur n'est pas Sync,
        // le booléen l'est).
        let parent_scripting = eng.is_some();
        let mut async_mods = Vec::new();
        let mut frames = Vec::new();
        fetch_wave(&self.transport, self.max_inflight, jobs, |tag, res| {
            // CAP-185 : `load`/`error` de fin de feuille —
            // enregistré sur toutes les issues (fetch refusé,
            // non-2xx, sécurité/SRI) ; livré au commit.
            let Ok(resp) = res else {
                if let SubTag::Css(i) = tag {
                    let ord = self.sheet_urls.get(i).map_or(0, |(.., lord)| *lord);
                    self.link_events.push((ord, false));
                }
                return;
            };
            if !(200..300).contains(&resp.status) {
                if let SubTag::Css(i) = tag {
                    let ord = self.sheet_urls.get(i).map_or(0, |(.., lord)| *lord);
                    self.link_events.push((ord, false));
                }
                return;
            }
            match tag {
                SubTag::Css(i) => {
                    let (su, sri, .., lord) = &self.sheet_urls[i];
                    let ok = subresp_security_allows(&self.url, su, FetchDest::Style, &resp)
                        && sri_allows(sri.as_deref(), &resp.body);
                    if ok {
                        self.css_slots[i] = Some(decode_stylesheet(&resp.body, &resp.headers));
                    }
                    self.link_events.push((*lord, ok));
                }
                SubTag::Script(slot) => {
                    let item = &self.scripts[slot_items[slot]];
                    let sec_ok = item.src.as_ref().is_none_or(|u| {
                        subresp_security_allows(&self.url, u, FetchDest::Script, &resp)
                    });
                    let sri_ok = sri_allows(item.integrity.as_deref(), &resp.body);
                    tracing::debug!(
                        tab = self.tab,
                        src = ?item.src.as_ref().map(Url::as_str),
                        module = item.module,
                        sec_ok,
                        sri_ok,
                        "sous-ressource script"
                    );
                    if sec_ok && sri_ok {
                        // Étape 53b : charset déclaré
                        // (classique) → BOM → UTF-8.
                        let text = decode_subresource(&resp.body, item.charset.as_deref());
                        if item.exec == ScriptExec::Arrival {
                            if item.module {
                                if let Some(u) = &item.src {
                                    async_mods.push((u.clone(), text));
                                }
                            } else {
                                eval_arrival(&mut eng, &text, item.el_ord);
                            }
                        } else {
                            self.script_srcs[slot] = Some(text);
                        }
                    }
                }
                SubTag::Img(u) => {
                    if subresp_security_allows(&self.url, &u, FetchDest::Image, &resp) {
                        self.img_bodies.insert(u.as_str(), resp.body);
                    }
                }
                SubTag::Frame(i) => {
                    let (u, _, sandbox) = &self.frame_urls[i];
                    collect_frame(
                        &self.url,
                        &self.cookies,
                        u,
                        &resp,
                        *sandbox,
                        parent_scripting,
                        &mut frames,
                    );
                } // `referrerpolicy`/`sandbox` vivent dans la
                  // `Request`/le `SubFrame`, pas le routage.
            }
        })
        .await;
        // `srcdoc` (CAP-142) : document inline — le HTML de
        // l'attribut est le sous-contexte (`about:srcdoc`),
        // aucune requête n'a été émise ni attendue.
        for (html, sandbox) in &self.srcdoc_frames {
            frames.push(store_srcdoc_frame(
                &self.url,
                html,
                *sandbox,
                frame_scripting(parent_scripting, *sandbox),
            ));
        }
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
            env: super::apply::viewport_env(self.viewport, self.color_scheme, self.scripting),
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
            let Some(base) = self.sheet_urls.get(i).map(|(u, ..)| u.clone()) else {
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
            let Ok(url) = Url::resolve(base, &imp.url)
                .map(|u| explorer_net::csp::csp_upgrade(&self.csp, &u).unwrap_or(u))
            else {
                return;
            };
            if !matches!(url.scheme(), "http" | "https") || !ctx.seen.insert(url.to_string()) {
                return;
            }
            // CSP `style-src` (CAP-145).
            if !explorer_net::csp::csp_allows(&self.csp, &self.url, FetchDest::Style, &url) {
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
                    None,
                ))
                .await
            else {
                return;
            };
            if !(200..300).contains(&resp.status) {
                return;
            }
            // En-têtes de sécurité de réponse (étape 134) —
            // même règle que la vague (dest `style`).
            if !subresp_security_allows(&self.url, &url, FetchDest::Style, &resp) {
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
        let env = super::apply::viewport_env(self.viewport, self.color_scheme, self.scripting);
        let mut left = self.max_images.saturating_sub(self.img_urls.len());
        let mut seen = HashSet::new();
        let mut decl_urls = Vec::new();
        for (i, slot) in self.css_slots.iter().enumerate() {
            let Some(text) = slot else { continue };
            let base = self
                .sheet_urls
                .get(i)
                .map_or_else(|| self.url.clone(), |(u, ..)| u.clone());
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
            decl_urls.push((self.doc_base.clone(), bg, mask));
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
                let Ok(url) = Url::resolve(&base, &raw)
                    .map(|u| explorer_net::csp::csp_upgrade(&self.csp, &u).unwrap_or(u))
                else {
                    continue;
                };
                if !matches!(url.scheme(), "http" | "https") || !seen.insert(url.to_string()) {
                    continue;
                }
                // CSP `img-src` (CAP-145) — même règle que
                // le plan pour les fetchs tardifs.
                if !explorer_net::csp::csp_allows(&self.csp, &self.url, FetchDest::Image, &url) {
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
                        None,
                    ))
                    .await
                else {
                    continue;
                };
                if !(200..300).contains(&resp.status) {
                    continue;
                }
                if !subresp_security_allows(&self.url, &url, FetchDest::Image, &resp) {
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
        let env = super::apply::viewport_env(self.viewport, self.color_scheme, self.scripting);
        'faces: for face in eng.font_faces(&env) {
            if self.fonts.len() >= self.font_budget {
                break;
            }
            for src in &face.srcs {
                let FontSrc::Url(href) = src else {
                    continue;
                };
                let Ok(url) = Url::resolve(&self.doc_base, href)
                    .map(|u| explorer_net::csp::csp_upgrade(&self.csp, &u).unwrap_or(u))
                else {
                    continue;
                };
                // CSP `font-src` (CAP-145).
                if !explorer_net::csp::csp_allows(&self.csp, &self.url, FetchDest::Font, &url) {
                    continue;
                }
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
                if !subresp_security_allows(&self.url, &req.url, FetchDest::Font, &resp) {
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
        tracing::debug!(
            tab = self.tab,
            async_mods = self.async_mods.len(),
            post_modules = self
                .scripts
                .iter()
                .filter(|i| i.exec == ScriptExec::Post && i.module)
                .count(),
            "prefetch modules"
        );
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
                || (format!("inline:{i}"), self.doc_base.clone()),
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
                &self.csp,
                &base,
                &src,
                &self.import_specs,
                &mut graph,
            )
            .await;
            tracing::debug!(tab = self.tab, name = %name, sources = graph.sources.len(), "module prefetché");
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
    /// `mode:` de la requête (JS-C77) — `NoCors` cross se
    /// livre opaque sans `ACAO` ; `Cors` exige
    /// `cors_readable`.
    pub(crate) mode: explorer_net::http::FetchMode,
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
    /// `<script>` dynamique (CAP-169) — `Some` = la réponse
    /// est un corps à évaluer par
    /// [`Session::deliver_dyn_script`] (`load`/`error` sur
    /// l'élément), jamais une `NetResponse` de `fetch`/XHR.
    /// `handle` porte alors le `seq` moteur.
    pub(crate) script: Option<DynScriptMeta>,
    /// `<iframe>` dynamique (CAP-171) — `Some` = la réponse
    /// est un document enfant à installer par
    /// [`Session::commit_dyn_frame`] (realm + `load`/`error`
    /// sur l'élément), jamais une `NetResponse`. `handle`
    /// porte alors le `seq` moteur.
    pub(crate) frame: Option<DynFrameMeta>,
    /// `<img>` dynamique (CAP-172) — `Some` = le corps est
    /// rangé dans `img_bodies`/`img_decoded` puis `load`/
    /// `error` tiré sur l'élément par
    /// [`Session::commit_dyn_img`] — jamais une
    /// `NetResponse` de `fetch`/XHR. `handle` porte alors
    /// le `seq` moteur.
    pub(crate) img: Option<DynImgMeta>,
    /// `<link>` dynamique (CAP-173) — `Some` = le corps est
    /// vérifié (SRI + sécurité sous-ressource), décodé en
    /// feuille et rangé dans `linked_sheets` puis `load`/
    /// `error` tiré sur l'élément par
    /// [`Session::commit_dyn_link`]. `handle` porte alors
    /// le `seq` moteur.
    pub(crate) link: Option<DynLinkMeta>,
    /// `<script type="module">` dynamique (CAP-186) —
    /// `Some` = le graphe de modules est rempli (entrée +
    /// dépendances fetchées), à évaluer puis `load`/`error`
    /// sur l'élément par [`Session::deliver_dyn_module`].
    pub(crate) dyn_module: Option<DynModuleDone>,
    /// `import()` dynamique (JS-C100) — `Some` = le corps est
    /// livré au moteur via [`Session::commit_dyn_import`]
    /// (instancier+évaluer+résoudre la promesse pending) ;
    /// `handle` porte le `seq` moteur.
    pub(crate) import: Option<DynImportMeta>,
}

/// Livraison d'un `import()` dynamique fetché (JS-C100) —
/// `ctx`/`seq` retrouvent la promesse pending moteur ;
/// `url` sert au log/attribution.
pub(crate) struct DynImportMeta {
    /// Contexte moteur émetteur.
    pub(crate) ctx: u32,
    /// `dyn_import_seq` moteur — clé `pending_imports`.
    pub(crate) seq: u64,
    /// Corps de l'entrée (2xx) — `None` = fetch refusé/erreur.
    pub(crate) entry: Option<String>,
    /// Dépendances **statiques** du module livré — `(url
    /// résolue, source)` seedées dans `Engine::module_sources`
    /// (JS-C101 : `instantiate` des deps de l'entrée les
    /// trouve — le chargeur ne sert que le préfetch).
    pub(crate) sources: Vec<(String, String)>,
}

/// Livraison d'un `<script type="module">` dynamique
/// (CAP-186) — le corps de l'entrée est déjà vérifié par
/// le canal `script` classique (2xx, sécurité sous-
/// ressource, SRI) puis le graphe est rempli hors session
/// par `fetch_module_sources` ; `graph = None` = échec en
/// amont → `error` sur l'élément.
pub(crate) struct DynModuleDone {
    /// Contexte moteur émetteur (`JS_CTX_TOP` ou iframe).
    pub(crate) ctx: u32,
    /// `dyn_script_seq` miroir — retrouve l'élément cible.
    pub(crate) seq: u64,
    /// Clé de registre moteur (URL de l'entrée ou
    /// `inline:{seq}`).
    pub(crate) name: String,
    /// Graphe rempli (`sources` + `alias`) — `None` si le
    /// corps de l'entrée a été refusé en amont.
    pub(crate) graph: Option<ModuleGraph>,
}

/// Métadonnées de livraison d'un `<script>` dynamique
/// (CAP-169) — le corps est vérifié (SRI, sécurité
/// sous-ressource) puis décodé avant évaluation.
pub(crate) struct DynScriptMeta {
    /// Contexte moteur émetteur (`JS_CTX_TOP` ou iframe).
    pub(crate) ctx: u32,
    /// `dyn_script_seq` miroir — retrouve l'élément cible
    /// de `load`/`error`/`currentScript`.
    pub(crate) seq: u64,
    /// `type="module"` (CAP-186) — le corps vérifié alimente
    /// un graphe `fetch_module_sources` (éval `eval_module`),
    /// pas l'éval classique `deliver_dyn_script`.
    pub(crate) module: bool,
    /// SRI déclaré sur l'élément (`integrity`).
    pub(crate) integrity: Option<String>,
    /// `charset` déclaré (étape 53b).
    pub(crate) charset: Option<String>,
    /// Document émetteur — ancre `nosniff`/CORP.
    pub(crate) page: Url,
}

/// Métadonnées de livraison d'un `<iframe>` dynamique
/// (CAP-171) — le document est installé en sous-contexte
/// (realm + miroir) puis `load`/`error` part sur l'élément.
pub(crate) struct DynFrameMeta {
    /// Contexte moteur émetteur (`JS_CTX_TOP` ou iframe) —
    /// parent du sous-contexte créé.
    pub(crate) ctx: u32,
    /// `dyn_script_seq` miroir partagé — retrouve l'élément
    /// cible de `load`/`error`.
    pub(crate) seq: u64,
    /// Index du nœud parmi les `<iframe>` du miroir
    /// émetteur (ordre document) — `context_new(frame_ord)`
    /// pour `contentWindow`/`frameElement`.
    pub(crate) ord: u32,
    /// `sandbox` de l'élément (CAP-149) — scripting/origine
    /// opaque du sous-contexte en découlent.
    pub(crate) sandbox: Option<SandboxFlags>,
    /// URL résolue du `src` (déjà CSP/mixed-content filtrée
    /// à l'émission).
    pub(crate) url: Url,
    /// Document émetteur — `Referer`/`Origin`/ancre
    /// `frame-ancestors`/partition cookies.
    pub(crate) page: Url,
}

/// Métadonnées de livraison d'un `<img>` dynamique
/// (CAP-172) — le corps vérifié est rangé sous deux clés :
/// `img_bodies` (URL résolue — source du re-décodage
/// bfcache) et `img_decoded` (chaîne `src`/candidat —
/// re-matche `collect_images` après resynchro), puis
/// `load`/`error` part sur l'élément.
pub(crate) struct DynImgMeta {
    /// Contexte moteur émetteur (`JS_CTX_TOP` ou iframe).
    pub(crate) ctx: u32,
    /// `dyn_script_seq` miroir partagé — retrouve l'élément
    /// cible de `load`/`error`.
    pub(crate) seq: u64,
    /// Chaîne source choisie telle que `collect_images` la
    /// relira (`src` brut ou candidat `srcset`) — clé de
    /// `img_decoded`.
    pub(crate) src_key: String,
    /// URL résolue du fetch (déjà CSP/mixed-content filtrée
    /// à l'émission).
    pub(crate) url: Url,
    /// Document émetteur — ancre `nosniff`/CORP/`img-src`.
    pub(crate) page: Url,
}

/// Métadonnées de livraison d'un `<link>` dynamique
/// (CAP-173) — le corps vérifié (SRI `integrity`, sécurité
/// sous-ressource) est décodé puis rangé dans
/// `linked_sheets` (la cascade le reprend au prochain
/// apply) avant `load`/`error` sur l'élément.
pub(crate) struct DynLinkMeta {
    /// Contexte moteur émetteur (`JS_CTX_TOP` ou iframe).
    pub(crate) ctx: u32,
    /// `dyn_script_seq` miroir partagé — retrouve l'élément
    /// cible de `load`/`error`.
    pub(crate) seq: u64,
    /// SRI déclarée sur l'élément (`integrity`, CAP-163).
    pub(crate) integrity: Option<String>,
    /// Attribut `media` brut — le fetch a toujours lieu
    /// (spec), l'application est gated par
    /// `media_matches` à la livraison.
    pub(crate) media: Option<String>,
    /// URL résolue du `href` (déjà CSP/mixed-content
    /// filtrée à l'émission).
    pub(crate) url: Url,
    /// Document émetteur — ancre `nosniff`/CORP/`style-src`.
    pub(crate) page: Url,
}

/// Complétion d'une opération `crypto.subtle` (JS-C79) —
/// produite par une tâche `spawn_blocking` (le calcul vit dans
/// `explorer_net::crypto`, jamais dans le moteur), consommée
/// par [`Session::pump_net`] → `deliver_crypto_response`.
pub(crate) struct CryptoDone {
    /// Onglet émetteur.
    pub(crate) tab: TabId,
    /// Corrélation côté moteur (`CryptoResponse::handle`).
    pub(crate) handle: u64,
    /// `Ok(octets)` ou `Err(message)` → `OperationError`.
    pub(crate) result: Result<Vec<u8>, String>,
}

/// Livraison d'un événement SSE `EventSource` (JS-C92) —
/// produite par la tâche streamée `fetch_stream` (open aux
/// en-têtes, un `Message` par événement parsé, `Error` à la
/// coupure), consommée par [`Session::poll_net`] →
/// `deliver_es` — multi-coups contrairement à `NetDone`.
/// Paramètres d'une charge de source `Worker`/`importScripts`
/// (JS-C132) — packés pour `Session::spawn_worker_load`
/// (clippy `too_many_arguments`, même motif [`EsSpawnArgs`]).
pub(crate) struct WorkerLoadArgs<'a> {
    /// Onglet émetteur.
    pub tab: crate::TabId,
    /// Page du document top-level.
    pub page: &'a explorer_net::url::Url,
    /// Base de résolution du document.
    pub base: &'a explorer_net::url::Url,
    /// Spécificateur tel qu'écrit (`new Worker(spec)`).
    pub spec: &'a str,
    /// URL de la page émettrice (`hooks.page_url`).
    pub referrer: &'a str,
    /// Contexte émetteur.
    pub ctx: explorer_js::JsCtxId,
    /// Canal one-shot de réponse au thread bloqué en `load()`.
    pub reply: explorer_js::WorkerReply,
}

/// Paramètres d'ouverture d'un flux SSE (JS-C92) — packés
/// pour `Session::spawn_es` (clippy `too_many_arguments`).
pub(crate) struct EsSpawnArgs<'a> {
    pub tab: crate::TabId,
    pub page: &'a explorer_net::url::Url,
    pub base: &'a explorer_net::url::Url,
    pub handle: u64,
    pub url: &'a str,
    pub with_credentials: bool,
    pub ctx: explorer_js::JsCtxId,
}

pub(crate) struct EsDone {
    /// Onglet émetteur.
    pub(crate) tab: TabId,
    /// Contexte émetteur — la livraison vise son realm.
    pub(crate) ctx: explorer_js::JsCtxId,
    /// Corrélation côté moteur (`EsTransport::open`).
    pub(crate) handle: u64,
    /// `open`/`message`/`error` pour [`JsEngine::deliver_es_event`].
    pub(crate) ev: explorer_js::EsEvent,
}

/// Paramètres d'ouverture `WebSocket` (JS-C94) — packés
/// pour `Session::spawn_ws` (clippy `too_many_arguments`,
/// même motif [`EsSpawnArgs`]).
pub(crate) struct WsSpawnArgs<'a> {
    pub tab: crate::TabId,
    pub page: &'a explorer_net::url::Url,
    pub base: &'a explorer_net::url::Url,
    pub handle: u64,
    pub url: &'a str,
    pub protocols: &'a [String],
    pub ctx: explorer_js::JsCtxId,
}

/// Livraison d'un événement `WebSocket` (JS-C94) — produite
/// par la tâche `WsDial::open` (socket dédiée hors pool),
/// consommée par [`Session::poll_net`] → `deliver_ws`.
/// Multi-coups ; `Close`/`Error` = fin de flux.
pub(crate) struct WsDone {
    /// Onglet émetteur.
    pub(crate) tab: TabId,
    /// Contexte émetteur — la livraison vise son realm.
    pub(crate) ctx: explorer_js::JsCtxId,
    /// Corrélation côté moteur (`WsTransport::open`).
    pub(crate) handle: u64,
    /// Événement brut du dial — traduit en `WsEventKind`
    /// moteur dans `deliver_ws` (`Error` → `error` +
    /// `close 1006` spec « fail the connection »).
    pub(crate) ev: explorer_net::ws::WsEvent,
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

/// `CorsRequest` d'un `fetch`/`XHR` cross `mode:cors` (étape
/// 126 + JS-C77 + CAP-149) : en-têtes auteur non safelistés
/// (ou `Content-Type` hors types « form ») →
/// `unsafe_headers` déclenchant le preflight `OPTIONS` du
/// `CorsTransport` ; `origin` = origine de la page ou `null`
/// en contexte opaque.
pub(crate) fn cors_request_for(
    page: &Url,
    headers: &[(String, String)],
    opaque: bool,
    credentials: explorer_net::http::FetchCredentials,
) -> explorer_net::http::CorsRequest {
    let mut unsafe_headers: Vec<String> = headers
        .iter()
        .filter(|(n, _)| !forbidden_request_header(n))
        .filter(|(n, v)| {
            !cors_safelisted_header(n)
                || (n.trim().eq_ignore_ascii_case("content-type")
                    && !explorer_net::policy::cors_safelisted_content_type(
                        v.split(';').next().unwrap_or("").trim(),
                    ))
        })
        .map(|(n, _)| n.trim().to_ascii_lowercase())
        .collect();
    unsafe_headers.sort();
    unsafe_headers.dedup();
    explorer_net::http::CorsRequest {
        origin: if opaque {
            "null".to_string()
        } else {
            origin_header_value(page)
        },
        credentials,
        unsafe_headers,
    }
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
    sheet_urls: &[SheetFetch],
    scripts: &[ScriptItem],
    img_urls: &[Url],
    frame_urls: &[(Url, Option<ReferrerPolicy>, Option<SandboxFlags>)],
    page_policy: Option<ReferrerPolicy>,
) -> VecDeque<(SubTag, Request)> {
    let mut jobs = VecDeque::new();
    for (i, (u, _, cross, _lord)) in sheet_urls.iter().enumerate() {
        jobs.push_back((
            SubTag::Css(i),
            sub_request(base, u, FetchDest::Style, page_policy, *cross),
        ));
    }
    for item in scripts {
        if let (Some(u), Some(slot)) = (&item.src, item.slot) {
            tracing::debug!(src = %u, module = item.module, "script émis");
            jobs.push_back((
                SubTag::Script(slot),
                sub_request(base, u, FetchDest::Script, page_policy, item.crossorigin),
            ));
        }
    }
    for u in img_urls {
        jobs.push_back((
            SubTag::Img(u.clone()),
            sub_request(base, u, FetchDest::Image, page_policy, None),
        ));
    }
    for (i, (u, policy, _)) in frame_urls.iter().enumerate() {
        jobs.push_back((
            SubTag::Frame(i),
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
///
/// `cross_origin` (attribut `crossorigin`, CAP-167) : `Some`
/// → `fetch_mode: cors` ; cross-origin → `req.cors` armé
/// (origine canonique de la page, `unsafe_headers` vide —
/// GET safelisté) + `Origin:` explicite (spec CORS : émis
/// même sur GET) ; le `CorsTransport` exige alors `ACAO`
/// couvrant la page, refus = échec réseau. `SameOrigin`
/// same-origin → métadonnées `cors` seules (rien à enforcer).
pub(crate) fn sub_request(
    base: &Url,
    url: &Url,
    dest: FetchDest,
    policy: Option<ReferrerPolicy>,
    cross_origin: Option<FetchCredentials>,
) -> Request {
    let mut req = Request::get(url.clone());
    req.initiator = Some(Initiator::Subresource(base.clone()));
    req.fetch_dest = dest;
    req.referrer_policy = policy;
    if cross_origin.is_some() {
        req.fetch_mode = FetchMode::Cors;
    }
    for (n, v) in initiator_headers_policy(base, url, Method::Get, policy.unwrap_or_default()) {
        req.headers.push((n, v));
    }
    if let Some(creds) = cross_origin {
        if base.origin() != url.origin() {
            let origin = origin_header_value(base);
            req.cors = Some(explorer_net::http::CorsRequest {
                origin: origin.clone(),
                credentials: creds,
                unsafe_headers: Vec::new(),
            });
            req.headers.push(("origin".to_string(), origin));
        }
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
    let mut req = sub_request(base, url, FetchDest::Iframe, policy, None);
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
///
/// `async`/arrivée désordonnée (étape 51) : le corps reçu
/// est évalué immédiatement sur le moteur embarqué — un
/// échec est rapporté en `PipelineError`, jamais fatal.
fn eval_async_arrival(
    eng: &mut Option<Box<dyn JsEngine>>,
    src: &str,
    el_ord: usize,
    tab: TabId,
    events: &broadcast::Sender<CoreEvent>,
) {
    if let Some(e) = eng.as_mut() {
        // `name` = étiquette de pile (`at fn (inline:N:l:c)`)
        // — attribution des exceptions de scripts inline
        // (JS-C87, même esprit que `eval_named` des scripts
        // dynamiques).
        let label = format!("inline:{el_ord}");
        if let Err(err) = e.eval_script_el(explorer_js::JS_CTX_TOP, src, Some(&label), el_ord) {
            tracing::debug!(tab, error = %err, "script async en échec");
            let _ = events.send(CoreEvent::PipelineError {
                tab: Some(tab),
                message: err.to_string(),
            });
        }
    }
}

/// Tree-builder avec « scripting flag » (CAP-155) —
/// `<noscript>` = rawtext quand le JS tournera.
fn scripted_tree_builder(scripting: bool) -> TreeBuilder {
    let mut tb = TreeBuilder::new();
    tb.set_scripting(scripting);
    tb
}

/// Scripting effectif d'un sous-contexte (CAP-155 — spec
/// §13.2.3.5) : scripting du parent **et** `allow-scripts`
/// du sandbox — `<noscript>` n'est rawtext que si les
/// scripts de l'enfant tourneront (un cadre `sandbox` sans
/// `allow-scripts` doit afficher le fallback).
pub(crate) fn frame_scripting(parent: bool, sandbox: Option<SandboxFlags>) -> bool {
    parent && sandbox.is_none_or(|s| s.scripts)
}

/// Réception d'un `<iframe src>` : XFO/CSP
/// `frame-ancestors` (étape 134) peuvent interdire
/// l'embarquement — sinon le `SubFrame` est stocké
/// (scripting effectif calculé, CAP-155).
pub(crate) fn collect_frame(
    page: &Url,
    cookies: &CookieJar,
    u: &Url,
    resp: &Response,
    sandbox: Option<SandboxFlags>,
    parent_scripting: bool,
    frames: &mut Vec<SubFrame>,
) {
    if explorer_net::policy::frame_embedding_allowed(page, u, resp) {
        frames.push(store_frame(
            page,
            cookies,
            u,
            resp,
            sandbox,
            frame_scripting(parent_scripting, sandbox),
        ));
    }
}

/// `scripting` (CAP-155) — voir [`frame_scripting`].
pub(crate) fn store_frame(
    page: &Url,
    cookies: &CookieJar,
    u: &Url,
    resp: &Response,
    sandbox: Option<SandboxFlags>,
    scripting: bool,
) -> SubFrame {
    let partition = partition_key(page);
    for v in resp.headers_all("set-cookie") {
        let _ = cookies.set_from_header_ctx(u, v, partition.clone());
    }
    let text = decode_subresource(&resp.body, transport_charset(&resp.headers).as_deref());
    let dom = TreeBuilder::parse_scripting(&text, scripting);
    let title = extract_title(&dom);
    SubFrame {
        url: u.clone(),
        sandbox,
        srcdoc: None,
        dom,
        title,
        // Source gardée : le miroir JS de l'enfant est
        // re-parsé depuis elle au câblage du contexte
        // (étape 134) — pas de re-fetch pour le realm.
        html: text,
    }
}

/// Réception d'un document `<iframe srcdoc>` (CAP-142) :
/// le HTML de l'attribut est parsé tel quel — l'URL du
/// sous-contexte est `about:srcdoc` (schéma opaque,
/// origine opaque héritée à l'usage ; repli dégradé = URL
/// de la page si jamais le littéral échouait) ; aucun
/// `Set-Cookie` n'existe à capturer (pas de réponse). Le
/// contenu est conservé dans `srcdoc` — c'est la clé
/// d'association boîte ↔ contexte côté composite (deux
/// cadres au même HTML partagent le `SubFrame`, comme deux
/// `src` homonymes).
pub(crate) fn store_srcdoc_frame(
    page: &Url,
    html: &str,
    sandbox: Option<SandboxFlags>,
    scripting: bool,
) -> SubFrame {
    let dom = TreeBuilder::parse_scripting(html, scripting);
    let title = extract_title(&dom);
    SubFrame {
        url: Url::parse("about:srcdoc").unwrap_or_else(|_| page.clone()),
        sandbox,
        srcdoc: Some(html.to_string()),
        dom,
        title,
        html: html.to_string(),
    }
}

/// En-têtes de sécurité de réponse sur une sous-ressource
/// `no-cors` (étape 134) : `X-Content-Type-Options: nosniff`
/// (script/style → `Content-Type` déclaré correct, MIME
/// sniffing §5) + `Cross-Origin-Resource-Policy`
/// (`same-origin`/`same-site` restreignent à la page
/// initiatrice). `true` = la réponse est admise ; sinon la
/// sous-ressource est simplement abandonnée (jamais d'échec
/// propagé — même dégradation qu'un 404).
pub(crate) fn subresp_security_allows(
    page: &Url,
    url: &Url,
    dest: FetchDest,
    resp: &Response,
) -> bool {
    explorer_net::policy::nosniff_allows(dest, resp)
        && explorer_net::policy::corp_allows(page, url, resp)
}

/// SRI (CAP-163) : pas de métadonnée `integrity` → admis ;
/// déclarée et mismatch → corps abandonné (même surface
/// qu'un échec réseau — jamais évalué ni appliqué, spec).
pub(crate) fn sri_allows(meta: Option<&str>, body: &[u8]) -> bool {
    meta.is_none_or(|m| explorer_net::crypto::integrity_allows(m, body))
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
