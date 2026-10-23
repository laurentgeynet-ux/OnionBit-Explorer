// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Formulaires : contrôles, validation HTML, collecte
//! `collect_form_data` et encodages url-encoded/multipart.
use super::*;

use explorer_dom::{Dom, DomError, NodeId};
use explorer_net::http::{Initiator, Method, Request};
use explorer_net::url::Url;

/// Contrôle de soumission le plus proche (soi ou ancêtre) :
/// `<button>` sans `type="button|reset"` (défaut HTML =
/// submit) ou `<input type="submit|image">`.
pub(crate) fn submit_control(dom: &Dom, mut node: NodeId) -> Option<NodeId> {
    loop {
        match dom.tag_name(node) {
            Ok("button") => {
                let ty = dom
                    .attribute(node, "type")
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                if !matches!(ty.as_str(), "button" | "reset") {
                    return Some(node);
                }
            }
            Ok("input") => {
                let ty = dom
                    .attribute(node, "type")
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                if matches!(ty.as_str(), "submit" | "image") {
                    return Some(node);
                }
            }
            _ => {}
        }
        node = dom.parent(node).ok()??;
    }
}

/// `input[type=checkbox|radio]` le plus proche (soi ou
/// ancêtre). Un clic sur un `<label for=id>` résout le
/// contrôle nommé (délégation HTML standard).
pub(crate) fn toggle_control(dom: &Dom, mut node: NodeId) -> Option<NodeId> {
    loop {
        match dom.tag_name(node) {
            Ok("input") => {
                let ty = dom
                    .attribute(node, "type")
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                if matches!(ty.as_str(), "checkbox" | "radio") {
                    return Some(node);
                }
            }
            Ok("label") => {
                if let Ok(Some(f)) = dom.attribute(node, "for") {
                    if let Some(target) = find_by_id(dom, &f) {
                        if dom.tag_name(target).is_ok_and(|t| t == "input") {
                            return Some(target);
                        }
                    }
                }
            }
            _ => {}
        }
        node = dom.parent(node).ok()??;
    }
}

/// `<select>` le plus proche (soi ou ancêtre), `label for=`
/// résolu comme `toggle_control`.
pub(crate) fn select_control(dom: &Dom, mut node: NodeId) -> Option<NodeId> {
    loop {
        match dom.tag_name(node) {
            Ok("select") => return Some(node),
            Ok("label") => {
                if let Ok(Some(f)) = dom.attribute(node, "for") {
                    if let Some(target) = find_by_id(dom, &f) {
                        if dom.tag_name(target).is_ok_and(|t| t == "select") {
                            return Some(target);
                        }
                    }
                }
            }
            _ => {}
        }
        node = dom.parent(node).ok()??;
    }
}

/// Bascule l'état du contrôle : `checkbox` inverse son
/// `checked` ; `radio` se coche et décoche ses homonymes
/// (`name` identique) du même formulaire (ou du document
/// hors formulaire).
pub(crate) fn toggle_checked(dom: &mut Dom, ctrl: NodeId) -> Result<(), DomError> {
    let ty = dom.attribute(ctrl, "type")?.unwrap_or_default();
    if ty == "radio" {
        if dom.attribute(ctrl, "checked")?.is_some() {
            return Ok(()); // un radio coché ne se décoche pas au clic
        }
        dom.set_attribute(ctrl, "checked", "")?;
        let name = dom.attribute(ctrl, "name")?.unwrap_or_default();
        if !name.is_empty() {
            let scope = enclosing_form(dom, ctrl).unwrap_or_else(|| dom.document());
            let mut queue = vec![scope];
            while let Some(n) = queue.pop() {
                if n != ctrl && dom.tag_name(n).is_ok_and(|t| t == "input") {
                    let same = dom.attribute(n, "type")?.unwrap_or_default() == "radio"
                        && dom.attribute(n, "name")?.unwrap_or_default() == name;
                    if same {
                        dom.remove_attribute(n, "checked")?;
                    }
                }
                if let Ok(ch) = dom.children(n) {
                    queue.extend(ch);
                }
            }
        }
        return Ok(());
    }
    if dom.attribute(ctrl, "checked")?.is_some() {
        dom.remove_attribute(ctrl, "checked")?;
    } else {
        dom.set_attribute(ctrl, "checked", "")?;
    }
    Ok(())
}

/// Options du `<select>` : `selected` sur l'index choisi,
/// retiré des autres (radio-like).
pub(crate) fn choose_option(dom: &mut Dom, sel: NodeId, idx: usize) {
    for (i, o) in descendant_elements(dom, sel, "option")
        .into_iter()
        .enumerate()
    {
        if i == idx {
            let _ = dom.set_attribute(o, "selected", "");
        } else {
            let _ = dom.remove_attribute(o, "selected");
        }
    }
}

/// Attribut interne transportant `setCustomValidity()` du
/// miroir JS vers le DOM réel via la synchro : le miroir le
/// pose en `data-explorer-cv` (visible dans `outerHTML` —
/// déviation documentée ADR-0014), le moteur le lit ici.
pub(crate) const CUSTOM_VALIDITY_ATTR: &str = "data-explorer-cv";

/// Contrôles du formulaire qui échouent la validation de
/// contraintes (étape 54 — HTML §4.10.21.2) : `required`,
/// `minlength`, `maxlength`, `pattern`, `min`/`max`/`step`
/// (types numériques + `date`-like MVP = non validés,
/// déviation) et la validité custom posée par
/// `setCustomValidity`. Les non-candidats (`disabled`,
/// `readonly` sur un champ non textuel, boutons, `type=hidden`)
/// ne valident jamais.
pub(crate) fn invalid_controls(dom: &Dom, form: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut queue = vec![form];
    while let Some(n) = queue.pop() {
        let tag = dom.tag_name(n).unwrap_or("");
        if n != form
            && dom.element(n).is_ok()
            && is_control_candidate(dom, n, tag)
            && control_invalid_reason(dom, n, tag).is_some()
        {
            out.push(n);
        }
        if n == form || tag != "form" {
            if let Ok(ch) = dom.children(n) {
                queue.extend(ch.iter().rev());
            }
        }
    }
    out
}

/// `willValidate` du contrôle : `disabled`, boutons et
/// `type=hidden`/`submit`-like exclus (HTML §4.10.21.2 —
/// `barred from constraint validation`).
pub(crate) fn is_control_candidate(dom: &Dom, n: NodeId, tag: &str) -> bool {
    if dom.attribute(n, "disabled").ok().flatten().is_some() {
        return false;
    }
    match tag {
        "input" => {
            let ty = attr_lower(dom, n, "type");
            !matches!(
                ty.as_str(),
                "hidden" | "submit" | "image" | "reset" | "button" | "file"
            )
        }
        "textarea" | "select" => true,
        _ => false,
    }
}

/// Première raison d'invalidité du contrôle — `None` si
/// valide. L'ordre suit HTML : custom, required, longueurs,
/// pattern, bornes numériques, pas (`step`).
pub(crate) fn control_invalid_reason(dom: &Dom, n: NodeId, tag: &str) -> Option<&'static str> {
    if dom
        .attribute(n, CUSTOM_VALIDITY_ATTR)
        .ok()
        .flatten()
        .is_some_and(|m| !m.is_empty())
    {
        return Some("customError");
    }
    let (value, empty_required) = match tag {
        "input" => {
            let ty = attr_lower(dom, n, "type");
            match ty.as_str() {
                "checkbox" => (
                    String::new(),
                    dom.attribute(n, "checked").ok().flatten().is_none(),
                ),
                "radio" => (String::new(), !radio_group_checked(dom, n)),
                _ => (
                    input_value(dom, n, &ty),
                    input_value(dom, n, &ty).is_empty(),
                ),
            }
        }
        "textarea" => {
            let v = dom.text_content(n).unwrap_or_default();
            let e = v.is_empty();
            (v, e)
        }
        "select" => {
            // Invalide seulement si `required` et que
            // l'option choisie porte une valeur vide
            // (placeholder label option).
            let v = select_value(dom, n);
            let e = v.is_empty();
            (v, e)
        }
        _ => (String::new(), false),
    };
    if dom.attribute(n, "required").ok().flatten().is_some() && empty_required {
        return Some("valueMissing");
    }
    // Longueurs + pattern : champs textuels seulement —
    // `readonly` excepté pour required (HTML).
    let textual = tag == "textarea"
        || matches!(
            attr_lower(dom, n, "type").as_str(),
            "" | "text" | "search" | "tel" | "url" | "email" | "password"
        );
    if textual {
        let len = value.chars().count();
        if let Ok(min) = attr_lower(dom, n, "minlength").parse::<usize>() {
            if len < min && !value.is_empty() {
                return Some("tooShort");
            }
        }
        if let Ok(max) = attr_lower(dom, n, "maxlength").parse::<usize>() {
            if len > max {
                return Some("tooLong");
            }
        }
        if let Some(pat) = dom.attribute(n, "pattern").ok().flatten() {
            if !value.is_empty() && !pattern_full_match(&pat, &value) {
                return Some("patternMismatch");
            }
        }
    }
    // Bornes + pas numériques : `number`, `range` (toujours
    // borné), et les types à valeur `f64` parseable.
    if matches!(attr_lower(dom, n, "type").as_str(), "number" | "range") {
        if let Ok(v) = value.trim().parse::<f64>() {
            if let Some(min) = attr_opt(dom, n, "min").and_then(|m| m.parse::<f64>().ok()) {
                if v < min {
                    return Some("rangeUnderflow");
                }
            }
            if let Some(max) = attr_opt(dom, n, "max").and_then(|m| m.parse::<f64>().ok()) {
                if v > max {
                    return Some("rangeOverflow");
                }
            }
            // `step` : `(v - min) % step == 0`, base = `min`
            // sinon 0 (défaut `step=1`).
            let step = attr_opt(dom, n, "step").and_then(|s| s.parse::<f64>().ok());
            if let Some(step) = step.filter(|s| *s > 0.0) {
                let base = attr_opt(dom, n, "min")
                    .and_then(|m| m.parse::<f64>().ok())
                    .unwrap_or(0.0);
                let rem = ((v - base) / step).fract();
                if rem.abs() > 1e-9 && (rem.abs() - 1.0).abs() > 1e-9 {
                    return Some("stepMismatch");
                }
            }
        }
    }
    None
}

/// Un `<input type=radio>` du même groupe (même `name`,
/// même formulaire) est coché — `required` est satisfait
/// au niveau du groupe (HTML).
pub(crate) fn radio_group_checked(dom: &Dom, n: NodeId) -> bool {
    let Some(name) = attr_opt(dom, n, "name") else {
        return dom.attribute(n, "checked").ok().flatten().is_some();
    };
    // Ancêtre `<form>` le plus proche, sinon la racine du
    // document (un groupe hors formulaire reste groupé).
    let mut scope = n;
    while let Ok(Some(p)) = dom.parent(scope) {
        if dom.tag_name(p) == Ok("form") {
            scope = p;
            break;
        }
        scope = p;
    }
    descendant_elements(dom, scope, "input").iter().any(|&r| {
        attr_lower(dom, r, "type") == "radio"
            && attr_opt(dom, r, "name").as_deref() == Some(name.as_str())
            && dom.attribute(r, "checked").ok().flatten().is_some()
    })
}

/// `pattern` HTML : correspondance **entière** (la spec
/// enveloppe en `^(?:…)$`). Un pattern qui ne compile pas
/// ne contraint pas (HTML) — `regex` ne couvre pas tout le
/// dialecte JS (lookahead…) : déviation documentée.
pub(crate) fn pattern_full_match(pat: &str, value: &str) -> bool {
    regex::Regex::new(&format!("^(?:{pat})$")).is_ok_and(|re| re.is_match(value))
}

/// `(name, value)` des contrôles successful du formulaire
/// Champ sérialisable d'un formulaire (étape 54 —
/// `multipart/form-data` exige de distinguer les fichiers
/// des valeurs texte).
pub(crate) enum FormEntry {
    /// `(name, value)` — contrôle textuel.
    Text(String, String),
    /// `(name, filename, bytes)` — `input[type=file]` : le
    /// nom de fichier vient de l'attribut `value` (aucun
    /// sélecteur de fichier réel — déviation ADR-0014), un
    /// champ non choisi émet un fichier vide (HTML).
    File(String, String, Vec<u8>),
}

/// Champs « successful » du formulaire (HTML §4.10.21) :
/// nommés, non `disabled`, cases cochées seulement, boutons
/// uniquement le `submitter`. Déviations MVP : `form=`
/// externe, `<fieldset disabled>` et `<object>` non traités.
pub(crate) fn collect_form_data(
    dom: &Dom,
    form: NodeId,
    submitter: Option<NodeId>,
) -> Vec<FormEntry> {
    let mut out = Vec::new();
    let mut queue = vec![form];
    while let Some(n) = queue.pop() {
        let tag = dom.tag_name(n).unwrap_or("");
        if n != form
            && dom.element(n).is_ok()
            && dom.attribute(n, "disabled").ok().flatten().is_none()
        {
            match tag {
                "input" => {
                    let ty = attr_lower(dom, n, "type");
                    match ty.as_str() {
                        "checkbox" | "radio" => {
                            if dom.attribute(n, "checked").ok().flatten().is_some() {
                                push_named(dom, n, input_value(dom, n, &ty), &mut out);
                            }
                        }
                        "submit" | "image" | "button" | "reset" => {
                            if Some(n) == submitter {
                                push_named(dom, n, input_value(dom, n, &ty), &mut out);
                            }
                        }
                        "file" => {
                            if let Some(name) = attr_opt(dom, n, "name").filter(|s| !s.is_empty()) {
                                let filename = attr_opt(dom, n, "value").unwrap_or_default();
                                out.push(FormEntry::File(name, filename, Vec::new()));
                            }
                        }
                        _ => push_named(dom, n, input_value(dom, n, &ty), &mut out),
                    }
                }
                "textarea" => {
                    push_named(dom, n, dom.text_content(n).unwrap_or_default(), &mut out);
                }
                "select" => {
                    push_named(dom, n, select_value(dom, n), &mut out);
                }
                "button" => {
                    let ty = dom.attribute(n, "type").ok().flatten().unwrap_or_default();
                    if Some(n) == submitter && !matches!(ty.as_str(), "button" | "reset") {
                        push_named(
                            dom,
                            n,
                            dom.attribute(n, "value").ok().flatten().unwrap_or_default(),
                            &mut out,
                        );
                    }
                }
                _ => {}
            }
        }
        // Un `<form>` ne s'imbrigue pas : on ne descend pas
        // dans un formulaire enfant éventuel.
        if n == form || tag != "form" {
            if let Ok(ch) = dom.children(n) {
                queue.extend(ch.iter().rev());
            }
        }
    }
    out
}

/// Pousse `Text(name, value)` si le contrôle est nommé.
pub(crate) fn push_named(dom: &Dom, n: NodeId, value: String, out: &mut Vec<FormEntry>) {
    if let Some(name) = attr_opt(dom, n, "name").filter(|s| !s.is_empty()) {
        out.push(FormEntry::Text(name, value));
    }
}

/// Valeur courante d'un `<input>` : `value` ou `"on"` pour
/// checkbox/radio sans `value`, vide sinon.
pub(crate) fn input_value(dom: &Dom, n: NodeId, ty: &str) -> String {
    dom.attribute(n, "value").ok().flatten().unwrap_or_else(|| {
        if matches!(ty, "checkbox" | "radio") {
            "on".to_string()
        } else {
            String::new()
        }
    })
}

/// Valeur courante d'un `<select>` : `value` (ou texte) de
/// l'option `selected`, sinon de la première.
pub(crate) fn select_value(dom: &Dom, sel: NodeId) -> String {
    let opts = descendant_elements(dom, sel, "option");
    let sel_opt = opts
        .iter()
        .find(|&&o| dom.attribute(o, "selected").ok().flatten().is_some())
        .or_else(|| opts.first());
    sel_opt.map_or_else(String::new, |&o| {
        dom.attribute(o, "value")
            .ok()
            .flatten()
            .unwrap_or_else(|| dom.text_content(o).unwrap_or_default().trim().to_string())
    })
}

/// Encodage `application/x-www-form-urlencoded` : espace→`+`,
/// non-réservés conservés, le reste en `%XX`.
/// Encodage `application/x-www-form-urlencoded` : espace→`+`,
/// non-réservés conservés, le reste en `%XX`. Un `File` y
/// porte son nom de fichier (HTML).
pub(crate) fn form_url_encode(entries: &[FormEntry]) -> String {
    pub(crate) fn enc(s: &str) -> String {
        s.bytes()
            .map(|b| match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                    (b as char).to_string()
                }
                b' ' => "+".to_string(),
                _ => format!("%{b:02X}"),
            })
            .collect()
    }
    entries
        .iter()
        .map(|e| {
            let (k, v) = match e {
                FormEntry::Text(k, v) => (k, v),
                FormEntry::File(k, f, _) => (k, f),
            };
            format!("{}={}", enc(k), enc(v))
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// Corps `multipart/form-data` (RFC 7578 / HTML §4.10.21.6) :
/// chaque entrée est une partie `Content-Disposition:
/// form-data`, les fichiers portent `filename=` +
/// `Content-Type: application/octet-stream` (MIME réelle
/// indisponible sans picker — déviation documentée).
pub(crate) fn form_multipart_encode(entries: &[FormEntry], boundary: &str) -> Vec<u8> {
    pub(crate) fn esc(s: &str) -> String {
        s.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace(['\r', '\n'], "")
    }
    let mut body = Vec::new();
    for e in entries {
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        match e {
            FormEntry::Text(name, value) => {
                body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{}\"\r\n\r\n",
                        esc(name)
                    )
                    .as_bytes(),
                );
                body.extend_from_slice(value.as_bytes());
            }
            FormEntry::File(name, filename, bytes) => {
                body.extend_from_slice(
                    format!(
                        "Content-Disposition: form-data; name=\"{}\"; filename=\"{}\"\r\n\
                         Content-Type: application/octet-stream\r\n\r\n",
                        esc(name),
                        esc(filename)
                    )
                    .as_bytes(),
                );
                body.extend_from_slice(bytes);
            }
        }
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

/// Requête de soumission (HTML §4.10.21.7) partagée entre
/// `submit_form` (DOM réel) et `NavRequest::FormSubmit`
/// (miroir) : `POST` selon `enctype` (`multipart/form-data`
/// ou `urlencoded` — `text/plain` retombe sur urlencoded,
/// déviation ADR-0014), sinon `GET` en query.
pub(crate) fn form_request(
    base: &Url,
    action: &str,
    method: &str,
    enctype: &str,
    entries: &[FormEntry],
) -> Option<Request> {
    if method.eq_ignore_ascii_case("post") {
        let url = Url::resolve(base, action).ok()?;
        let (content_type, body) = if enctype.eq_ignore_ascii_case("multipart/form-data") {
            let boundary = multipart_boundary();
            (
                format!("multipart/form-data; boundary={boundary}"),
                form_multipart_encode(entries, &boundary),
            )
        } else {
            (
                "application/x-www-form-urlencoded".to_string(),
                form_url_encode(entries).into_bytes(),
            )
        };
        return Some(Request {
            method: Method::Post,
            url,
            headers: vec![("content-type".to_string(), content_type)],
            body: Some(body),
            // Soumission = navigation initiée par la page —
            // `SameSite=Lax` n'est pas envoyé sur POST
            // cross-site (méthode non sûre, étape 66).
            initiator: Some(Initiator::NavFrom(base.clone())),
            fetch_dest: explorer_net::http::FetchDest::Document,
            fetch_mode: explorer_net::http::FetchMode::NoCors,
            // `fetch_user` est marqué par le dispatch
            // (`try_submit` = geste ; `form.submit()` = non).
            fetch_user: false,
            // Navigation de formulaire — hors protocole CORS.
            cors: None,
        });
    }
    let qs = form_url_encode(entries);
    let target = if qs.is_empty() {
        action.to_string()
    } else {
        let sep = if action.contains('?') { '&' } else { '?' };
        format!("{action}{sep}{qs}")
    };
    let mut req = Request::get(Url::resolve(base, &target).ok()?);
    req.initiator = Some(Initiator::NavFrom(base.clone()));
    req.fetch_dest = explorer_net::http::FetchDest::Document;
    Some(req)
}

/// Requête d'un `NavRequest::FormSubmit` — les champs sont
/// déjà collectés côté miroir en `(name, value)` plats
/// (les fichiers y portent leur nom de fichier).
pub(crate) fn script_form_request(
    base: &Url,
    method: &str,
    action: &str,
    enctype: &str,
    fields: &[(String, String)],
) -> Option<Request> {
    let entries: Vec<FormEntry> = fields
        .iter()
        .map(|(n, v)| FormEntry::Text(n.clone(), v.clone()))
        .collect();
    form_request(base, action, method, enctype, &entries)
}

/// Frontière multipart pseudo-unique : 96 bits d'horloge +
/// compteur de process — suffisant sans dépendance `rand`,
/// le contenu qui la contiendrait la verrait invalide (RFC
/// recommande de vérifier, MVP documenté).
pub(crate) fn multipart_boundary() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    format!("----ExplorerFormBoundary{nanos:032x}")
}
