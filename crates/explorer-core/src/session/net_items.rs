// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Types du scheduler de sous-ressources : jobs de fetch,
//! graphe de modules, livraisons, requêtes CORS/cookies.
use super::*;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::Instant;

use explorer_css::{FontSrc, Origin, StyleEngine, StyleEnv};
use explorer_dom::Dom;
use explorer_html::TreeBuilder;
use explorer_js::JsEngine;
use explorer_net::error::NetResult;
use explorer_net::http::{HttpTransport, Initiator, Method, Request, Response};
use explorer_net::policy::initiator_headers;
use explorer_net::url::Url;
use explorer_render::decode_font;
use futures_util::stream::{FuturesUnordered, StreamExt};
use tokio::sync::broadcast;

/// Routage d'une complétion de sous-ressource (étape 50).
pub(crate) enum SubTag {
    /// Slot feuille liée (ordre document).
    Css(usize),
    /// Slot script externe (ordre document).
    Script(usize),
    /// `<img>` — clé `Url::as_str()` : la table images ne
    /// prend des `NodeId` qu'après la resynchro DOM.
    Img(Url),
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
        // Corps non affichable (étape 70) : bufferisé avec
        // borne et progression — jamais décodé en mojibake.
        // `DownloadOffered` n'est émis qu'au commit
        // (`offer_download`) : un « accept » de l'UI avant
        // l'enregistrement du pending ne peut pas échouer.
        if let Some(meta) = download_meta(&resp.headers) {
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
            return Ok(NavOutcome::Download(super::downloads::PendingDownload {
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
            }));
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
            {
                paint(tb.dom()).await;
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
    /// Plan de collecte — URLs résolues à `nav_apply`.
    pub(crate) sheet_urls: Vec<Url>,
    pub(crate) scripts: Vec<ScriptItem>,
    pub(crate) img_urls: Vec<Url>,
    /// CSS inline du document — nourrit la résolution
    /// `@font-face` (le DOM n'est pas accessible hors
    /// session).
    pub(crate) doc_css: String,
    /// Moteur du document — les `async` classiques sont
    /// évalués en ligne à réception pendant la vague.
    pub(crate) eng: Option<Box<dyn JsEngine>>,
    /// Réceptions de la vague — indexées par ordre document.
    pub(crate) css_slots: Vec<Option<String>>,
    pub(crate) script_srcs: Vec<Option<String>>,
    pub(crate) img_bodies: HashMap<String, Vec<u8>>,
    /// Corps des modules `async` arrivés (ordre d'arrivée).
    pub(crate) async_mods: Vec<(Url, String)>,
    /// Faces `@font-face` téléchargées et décodées —
    /// `family`, `weight`, `italic`, données brutes.
    pub(crate) fonts: Vec<(String, u16, bool, Vec<u8>)>,
    /// Modules à évaluer au commit — arrival puis post.
    pub(crate) arrival_evals: Vec<ModEval>,
    pub(crate) post_evals: Vec<ModEval>,
    /// Bornes recopiées de la config session.
    pub(crate) max_inflight: usize,
    pub(crate) max_modules: usize,
    pub(crate) font_budget: usize,
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
        let jobs = sub_jobs(&self.url, &self.sheet_urls, &self.scripts, &self.img_urls);
        let mut eng = self.eng.take();
        let mut async_mods = Vec::new();
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
            }
        })
        .await;
        self.eng = eng;
        self.async_mods = async_mods;
        self.fetch_fonts().await;
        self.prefetch_modules().await;
    }

    /// Fetch + décode des `@font-face` hors session (étape
    /// 53d — ex-`fetch_webfonts` déplacé) : faces résolues
    /// sur `doc_css` + feuilles liées téléchargées, chaque
    /// source `url()` essayée dans l'ordre (première qui
    /// décode gagne) ; `register` au commit borne le
    /// registre. Séquentiel — `font_budget` vus.
    pub(crate) async fn fetch_fonts(&mut self) {
        let mut eng = StyleEngine::new();
        eng.add_css(&self.doc_css.clone(), Origin::Author);
        let mut linked = String::new();
        for t in self.css_slots.iter().flatten() {
            linked.push_str(t);
            linked.push('\n');
        }
        eng.add_css(&linked, Origin::Author);
        'faces: for face in eng.font_faces(&StyleEnv::default()) {
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
                &base,
                &src,
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
    /// est conditionnée à `Access-Control-Allow-Origin`.
    pub(crate) cross: bool,
    /// Origine canonique de la page émettrice (check ACAO).
    pub(crate) page_origin: String,
    /// URL de la requête — capture `Set-Cookie` (same-origin
    /// uniquement, `credentials` cross-origin hors MVP).
    pub(crate) req_url: Url,
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

/// Verbe « safelisté » CORS — une requête cross-origin avec
/// un autre verbe exigerait un preflight `OPTIONS` (hors MVP).
pub(crate) const fn cors_simple_method(m: Method) -> bool {
    matches!(m, Method::Get | Method::Head | Method::Post)
}

/// En-tête safelisté CORS — une requête cross-origin portant
/// un autre en-tête exigerait un preflight : bloquée au MVP.
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
) -> VecDeque<(SubTag, Request)> {
    let mut jobs = VecDeque::new();
    for (i, u) in sheet_urls.iter().enumerate() {
        jobs.push_back((SubTag::Css(i), sub_request(base, u)));
    }
    for item in scripts {
        if let (Some(u), Some(slot)) = (&item.src, item.slot) {
            jobs.push_back((SubTag::Script(slot), sub_request(base, u)));
        }
    }
    for u in img_urls {
        jobs.push_back((SubTag::Img(u.clone()), sub_request(base, u)));
    }
    jobs
}

/// Requête GET de sous-ressource : `Referer`/`Origin` calculés
/// à l'émission (`initiator_headers`, strict-origin-when-
/// cross-origin) — le mixed-content est filtré en amont.
pub(crate) fn sub_request(base: &Url, url: &Url) -> Request {
    let mut req = Request::get(url.clone());
    req.initiator = Some(Initiator::Subresource(base.clone()));
    for (n, v) in initiator_headers(base, url, Method::Get) {
        req.headers.push((n, v));
    }
    req
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
/// corps de module — `module_specifiers` (explorer-jscore, JS-C5)
/// + résolution contre l'URL du module parent.
pub(crate) fn module_deps(src: &str, referrer: &Url) -> Vec<(String, Url)> {
    explorer_js::module_specifiers(src)
        .into_iter()
        .filter_map(|spec| Url::resolve(referrer, &spec).ok().map(|u| (spec, u)))
        .collect()
}
