// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Triage moteur — `engine.anomalies` (étape 248). Un dump
//! brut dit « tout l'état » ; un agent a besoin d'indices
//! bornés. Cette passe heuristique croise la marche
//! sémantique (`snapshot_entries`), la géométrie posée
//! (`geometry_report`), la console pipeline et les compteurs
//! de santé du moteur pour rendre des **suspects** classés —
//! chacun porte une `ref` actionnable quand il est lié à un
//! nœud. Jamais un verdict : c'est l'index de la
//! bibliothèque, la preuve reste au verbe suivant
//! (`engine.layout`, `engine.cascade`…).

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};

use crate::proto::RpcError;
use crate::state::{AgentConn, AgentShared};
use explorer_core::{SnapshotEntry, TabId};

/// Suspects rendus par défaut — borne nommée (règle 13),
/// surcharge `params.max`.
const ANOMALY_MAX: usize = 20;
/// Boîte sous ce côté = quasi-invisible pour l'utilisateur.
const TINY_PX: f32 = 1.0;
/// Marge (px CSS) avant de déclarer une boîte « hors
/// document » — évite le bruit des arrondis de bord.
const OFF_DOC_MARGIN: f32 = 8.0;
/// Profondeur DOM pathologique — au-delà, soupçon de
/// structure hostile ou de bug de boîtes imbriquées.
const DEEP_NEST: u16 = 48;
/// Âge d'un paint en vol au-delà duquel il est suspect
/// (même seuil que l'escalade B-26).
const PAINT_STALE_MS: u64 = 30_000;
/// Entrées console lues pour le triage.
const CONSOLE_SCAN: usize = 512;
/// Taille du message console cité dans le suspect.
const CONSOLE_CITE: usize = 120;

/// Borne de suspects publique — la docstring du paramètre
/// `max` l'annonce.
#[must_use]
pub(crate) fn default_max() -> usize {
    ANOMALY_MAX
}

/// Un élément est « interactif » si l'utilisateur peut y
/// agir — même lecture que la couche sémantique, sans la
/// dépendance aux rôles ARIA.
fn interactive(e: &SnapshotEntry) -> bool {
    match e.tag.as_str() {
        "a" => e.attr("href").is_some(),
        "button" | "select" | "textarea" => true,
        "input" => !matches!(e.attr("type"), Some("hidden")),
        _ => e.attr("contenteditable").is_some() || e.attr("onclick").is_some(),
    }
}

/// Identifiant court d'un nœud pour le détail humain :
/// `#id`, sinon `.class`, sinon texte tronqué.
fn label(e: &SnapshotEntry) -> String {
    if let Some(id) = e.attr("id") {
        return format!("#{id}");
    }
    if let Some(c) = e.attr("class").and_then(|s| s.split_whitespace().next()) {
        return format!(".{c}");
    }
    e.text.chars().take(24).collect()
}

/// `engine.anomalies{tab, max?}` — suspects bornés et
/// classés. Lecture seule : réinsère un orphelin garé avant
/// les sondes pour ne pas flaguer un état transitoire.
///
/// # Errors
/// [`RpcError`] si l'onglet est inconnu ou le document
/// momentanément détaché (apply en vol).
pub(crate) async fn run(
    st: &Arc<AgentShared>,
    conn: &mut AgentConn,
    tab: TabId,
    max: usize,
) -> Result<Value, RpcError> {
    let (entries, report, health) = {
        let mut s = st.session.lock().await;
        s.reclaim_orphaned_paint(tab);
        let entries = s.snapshot_entries(tab).map_err(RpcError::from)?;
        let report = s.geometry_report(tab);
        let health = s.pipeline_report(tab);
        (entries, report, health)
    };
    let rev = st.rev(tab);
    let boxes: HashMap<u32, (f32, f32, f32, f32)> =
        report.as_ref().map_or_else(HashMap::new, |r| {
            r.boxes
                .iter()
                .map(|b| (b.node, (b.x, b.y, b.w, b.h)))
                .collect()
        });
    let mut suspects: Vec<(u8, Value)> = Vec::new();
    for e in &entries {
        entry_suspects(conn, tab, rev, e, &boxes, report.as_ref(), &mut suspects);
    }

    // Santé du moteur — les « anomalies » peuvent être des
    // bugs du navigateur, pas de la page : c'est le propos
    // de l'introspection.
    if let Some(h) = &health {
        health_suspects(h, &mut suspects);
    }
    // Console pipeline — chaque entrée est une erreur par
    // construction (ring PipelineError).
    let errs = st.console_entries(tab, CONSOLE_SCAN);
    if !errs.is_empty() {
        suspects.push((
            3,
            json!({
                "kind": "console_errors",
                "count": errs.len(),
                "last": errs
                    .last()
                    .map(|m| m.chars().take(CONSOLE_CITE).collect::<String>()),
                "detail": "console.get pour le détail complet",
            }),
        ));
    }

    suspects.sort_by_key(|(rank, _)| *rank);
    let out: Vec<Value> = suspects.into_iter().take(max).map(|(_, v)| v).collect();
    Ok(json!({
        "suspects": out,
        "scanned": {
            "nodes": entries.len(),
            "boxes": boxes.len(),
            "console_errors": errs.len(),
        },
        "rev": rev,
        "note": "suspects heuristiques — indices bornés, pas verdicts",
    }))
}

/// Suspects de santé moteur — orphelin garé, apply mort en
/// vol, paint figé. Rang 0 : un moteur malade peut produire
/// toutes les anomalies d'affichage en aval.
fn health_suspects(h: &Value, suspects: &mut Vec<(u8, Value)>) {
    if h["state"]["orphan_parked"].as_bool() == Some(true) {
        suspects.push((
            0,
            json!({
                "kind": "orphan_parked",
                "detail": "payload paint orpheline garée — détenteur mort en vol, wait/frame la réclamera",
            }),
        ));
    }
    let stuck = h["counters"]["apply_started_without_fin"]
        .as_u64()
        .unwrap_or(0);
    if stuck > 0 {
        suspects.push((
            0,
            json!({
                "kind": "apply_stuck",
                "count": stuck,
                "detail": format!("{stuck} apply engagé(s) sans fin — peinture possiblement morte en vol"),
            }),
        ));
    }
    let age = h["counters"]["paint_in_flight_age_ms"]
        .as_u64()
        .unwrap_or(0);
    if age > PAINT_STALE_MS {
        suspects.push((
            0,
            json!({
                "kind": "paint_stale",
                "age_ms": age,
                "detail": format!("peinture en vol âgée de {age} ms — seuil escalade B-26"),
            }),
        ));
    }
}

/// Suspects liés à un nœud — interactif sans boîte,
/// quasi-invisible, posé hors document, imbrication
/// pathologique. La `ref` est allouée sur la révision
/// courante pour que l'agent puisse enchaîner
/// (`engine.layout`, `query.select`…).
fn entry_suspects(
    conn: &mut AgentConn,
    tab: TabId,
    rev: u64,
    e: &SnapshotEntry,
    boxes: &HashMap<u32, (f32, f32, f32, f32)>,
    report: Option<&explorer_core::GeometryReport>,
    suspects: &mut Vec<(u8, Value)>,
) {
    let gbox = boxes.get(&e.index);
    if interactive(e) {
        let Some(&(bx, by, bw, bh)) = gbox else {
            // Rang 0 — interactif sans boîte : le suspect
            // canonique du « 0×0 » (display:none, ancêtre
            // non boxifié, élément hors arbre).
            suspects.push((
                0,
                json!({
                    "kind": "interactive_no_box",
                    "ref": conn.handles.alloc(tab, e.index, rev),
                    "tag": e.tag,
                    "node": label(e),
                    "depth": e.depth,
                    "detail": "interactif sans boîte posée — display:none, ancêtre non boxifié ou hors arbre",
                }),
            ));
            return;
        };
        if bw < TINY_PX || bh < TINY_PX {
            suspects.push((
                1,
                json!({
                    "kind": "tiny_box",
                    "ref": conn.handles.alloc(tab, e.index, rev),
                    "tag": e.tag,
                    "node": label(e),
                    "rect": {"x": bx, "y": by, "w": bw, "h": bh},
                    "detail": "interactif quasi-invisible (<1px)",
                }),
            ));
        }
    }
    if let Some(&(bx, by, bw, bh)) = gbox {
        let (dw, dh) = report.map_or((0.0, 0.0), |r| (r.width, r.height));
        let off = bx < -OFF_DOC_MARGIN
            || by < -OFF_DOC_MARGIN
            || bx > dw + OFF_DOC_MARGIN
            || by > dh + OFF_DOC_MARGIN;
        if off {
            suspects.push((
                2,
                json!({
                    "kind": "off_document",
                    "ref": conn.handles.alloc(tab, e.index, rev),
                    "tag": e.tag,
                    "node": label(e),
                    "rect": {"x": bx, "y": by, "w": bw, "h": bh},
                    "detail": format!("boîte hors document {dw}×{dh} — overflow ou ancre fautive"),
                }),
            ));
        }
    }
    if e.depth >= DEEP_NEST {
        suspects.push((
            4,
            json!({
                "kind": "deep_nesting",
                "ref": conn.handles.alloc(tab, e.index, rev),
                "tag": e.tag,
                "node": label(e),
                "depth": e.depth,
                "detail": format!("profondeur {} — structure pathologique", e.depth),
            }),
        ));
    }
}
