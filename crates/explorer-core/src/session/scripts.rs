// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Collecte des `<script>` du document — ordre, types
//! classiques/module et items d'évaluation.

use explorer_dom::Dom;
use explorer_net::http::Request;
use explorer_net::policy::is_mixed_content;
use explorer_net::url::Url;

/// Types `type=` exécutés comme script **classique** (HTML —
/// sous-ensemble des « JavaScript MIME types » usuels ; le
/// type absent/vaut `""` par défaut).
pub(crate) const CLASSIC_SCRIPT_TYPES: &[&str] = &[
    "",
    "text/javascript",
    "application/javascript",
    "text/ecmascript",
    "application/ecmascript",
    "text/jscript",
];

/// Phase d'exécution d'un `<script>` (étape 50) — l'ordre
/// document est conservé à l'intérieur de chaque file.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScriptExec {
    /// Classique (inline ou `src`) : file « parse » — ordre
    /// document, évalué sitôt sa source disponible (le parse
    /// one-shot a déjà fini : approximation « parser-blocking »
    /// documentée — le script voit le document complet).
    Parse,
    /// `defer` ou module non-`async` : file post-parse, ordre
    /// document.
    Post,
    /// `async` (classique ou module) : à réception — le
    /// classique est évalué en ligne dans la vague, le module
    /// est bufferisé et évalué avant la file post-parse.
    Arrival,
}

/// Balise `<script>` exécutable, collectée en ordre document
/// (étape 50).
pub(crate) struct ScriptItem {
    /// `src` résolue — `Some` = script externe.
    pub(crate) src: Option<Url>,
    /// Corps inline (classique ou module).
    pub(crate) inline: Option<String>,
    /// `type="module"`.
    pub(crate) module: bool,
    /// File d'exécution.
    pub(crate) exec: ScriptExec,
    /// `charset` déclaré sur la balise (script classique
    /// externe seulement — ignoré par spec pour les modules,
    /// toujours UTF-8 ; étape 53b).
    pub(crate) charset: Option<String>,
    /// Slot de réception du corps fetché (index dans
    /// `script_srcs`) — `None` pour les inline.
    pub(crate) slot: Option<usize>,
}

/// `<script>` exécutables du document, ordre document —
/// `src` résolues contre `base` (`<base href>` ou URL du
/// document, étape 135), bornés `max_src`, mixed-content
/// filtré contre `page` (l'ancre de sécurité reste l'URL
/// du document, pas la base). `scripting = false` (pas de
/// fabrique JS) → vide : scripting désactivé = aucun fetch
/// de script (spec HTML). Sous CSP `script-src` (CAP-145) :
/// un `src` doit matcher une source, un inline exige
/// `'unsafe-inline'` ou `nonce=` valide — balise sinon
/// inerte, non collectée.
pub(crate) fn collect_scripts(
    dom: &Dom,
    page: &Url,
    base: &Url,
    scripting: bool,
    max_src: usize,
    csp: &[explorer_net::csp::CspPolicy],
) -> Vec<ScriptItem> {
    let mut out = Vec::new();
    if !scripting {
        return out;
    }
    let mut slots = 0usize;
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "script") {
            let ty = dom
                .attribute(n, "type")
                .ok()
                .flatten()
                .map_or_else(String::new, |t| t.trim().to_ascii_lowercase());
            let module = ty == "module";
            if module || CLASSIC_SCRIPT_TYPES.contains(&ty.as_str()) {
                // `nomodule` n'a d'effet que sur les classiques.
                let nomodule = dom.attribute(n, "nomodule").ok().flatten().is_some();
                if module || !nomodule {
                    let src_attr = dom
                        .attribute(n, "src")
                        .ok()
                        .flatten()
                        .filter(|s| !s.trim().is_empty());
                    if let Some(raw) = src_attr {
                        // Externe : `async` l'emporte sur `defer`
                        // (spec — les deux présents = async).
                        let async_ = dom.attribute(n, "async").ok().flatten().is_some();
                        let defer = !async_ && dom.attribute(n, "defer").ok().flatten().is_some();
                        let resolved = Url::resolve(base, raw.trim())
                            .ok()
                            .filter(|u| !is_mixed_content(page, u))
                            .filter(|u| {
                                explorer_net::csp::csp_allows(
                                    csp,
                                    page,
                                    explorer_net::http::FetchDest::Script,
                                    u,
                                )
                            });
                        if let (Some(u), true) = (resolved, slots < max_src) {
                            out.push(ScriptItem {
                                src: Some(u),
                                inline: None,
                                charset: if module {
                                    None
                                } else {
                                    dom.attribute(n, "charset").ok().flatten()
                                },
                                module,
                                exec: if async_ {
                                    ScriptExec::Arrival
                                } else if module || defer {
                                    ScriptExec::Post
                                } else {
                                    ScriptExec::Parse
                                },
                                slot: {
                                    let s = slots;
                                    slots += 1;
                                    Some(s)
                                },
                            });
                        }
                        // Hors borne / irrésoluble / bloqué :
                        // balise inerte, non collectée.
                    } else {
                        let text = dom.text_content(n).unwrap_or_default();
                        let nonce = dom.attribute(n, "nonce").ok().flatten();
                        let inline_ok = explorer_net::csp::csp_inline_allowed(
                            csp,
                            explorer_net::csp::InlineKind::Script,
                            nonce.as_deref(),
                        );
                        if !text.trim().is_empty() && inline_ok {
                            // Inline : `async`/`defer` sans effet —
                            // classique = file parse, module = post.
                            out.push(ScriptItem {
                                src: None,
                                inline: Some(text),
                                charset: None,
                                module,
                                exec: if module {
                                    ScriptExec::Post
                                } else {
                                    ScriptExec::Parse
                                },
                                slot: None,
                            });
                        }
                    }
                }
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    out
}

/// Corps des `<script type="importmap">` du document, ordre
/// document (issue #6) : JSON bruts poussés au moteur via
/// `set_import_map` **avant** le premier `eval_module`, et
/// leur `imports` parsé sert au prefetch (`module_deps`
/// réécrit les bare specifiers avant `Url::resolve` — sans
/// ça, `import 'react'` se résoudrait en relatif sur la
/// page référente et fetcherait une URL jamais voulue).
/// Approximation documentée : toutes les maps s'appliquent
/// à tous les modules de la page (la spec n'honore que les
/// maps antérieures au premier chargement de module —
/// l'ordre exact n'importe pas aux sites observés).
pub(crate) fn collect_import_maps(dom: &Dom) -> Vec<String> {
    let mut out = Vec::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.tag_name(n).is_ok_and(|t| t == "script") {
            let ty = dom
                .attribute(n, "type")
                .ok()
                .flatten()
                .map_or_else(String::new, |t| t.trim().to_ascii_lowercase());
            if ty == "importmap" {
                let text = dom.text_content(n).unwrap_or_default();
                if !text.trim().is_empty() {
                    out.push(text);
                }
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    out
}

/// `imports` d'une import map : paires `(clé, cible)` dans
/// l'ordre du document JSON — `scopes` ignoré, valeurs non
/// string sautées, JSON invalide → vide (tolérance miroir,
/// même contrat que `JsEngine::set_import_map`).
pub(crate) fn parse_import_map(json: &str) -> Vec<(String, String)> {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .and_then(|v| v.get("imports")?.as_object().cloned())
        .map(|obj| {
            obj.into_iter()
                .filter_map(|(k, t)| t.as_str().map(|t| (k, t.to_string())))
                .collect()
        })
        .unwrap_or_default()
}

/// Fusion additive des maps d'une page (spec « merge » — la
/// première cible d'une clé l'emporte, comme le moteur).
pub(crate) fn merge_import_maps(maps: &[String]) -> Vec<(String, String)> {
    let mut merged: Vec<(String, String)> = Vec::new();
    for json in maps {
        for (k, t) in parse_import_map(json) {
            if merged.iter().all(|(ek, _)| *ek != k) {
                merged.push((k, t));
            }
        }
    }
    merged
}

/// Le document porte-t-il un handler inline `on*` (JS-C45) ?
/// Une page sans `<script>` mais avec `onclick=` doit quand
/// même obtenir un moteur — sinon ses handlers sont lettre
/// morte. Les noms d'attributs sont lowercase depuis le
/// tokenizer ; `len > 2` écarte l'attribut `on` nu. Un faux
/// positif (`once`, `only`…) ne coûte qu'un moteur inutilisé.
pub(crate) fn has_inline_handlers(dom: &Dom) -> bool {
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if let Ok(attrs) = dom.attributes(n) {
            if attrs
                .iter()
                .any(|a| a.name.len() > 2 && a.name.starts_with("on"))
            {
                return true;
            }
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    false
}

/// Corps d'un `ScriptItem` : inline direct, externe depuis son
/// slot (`None` = fetch en échec → le script est sauté).
pub(crate) fn item_source<'a>(item: &'a ScriptItem, srcs: &'a [Option<String>]) -> Option<&'a str> {
    if let Some(t) = &item.inline {
        return Some(t);
    }
    item.slot
        .and_then(|s| srcs.get(s).and_then(|o| o.as_deref()))
}

/// Intention de navigation script résolue — produite par
/// [`Session::next_script_nav`], appliquée par
/// [`Session::drain_script_nav`].
pub(crate) enum ScriptNav {
    /// `location.*` : URL absolue déjà résolue.
    To(Url),
    /// `history.go/back/forward` : déplacement relatif
    /// (0 = rechargement de l'entrée courante).
    Go(isize),
    /// `form.submit()`/`requestSubmit()` : requête déjà
    /// construite (méthode + corps + en-têtes — étape 54).
    /// `Box` : `Request` est la variante large de l'enum.
    Submit(Box<Request>),
}
