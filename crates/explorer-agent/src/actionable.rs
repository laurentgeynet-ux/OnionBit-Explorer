// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Actionabilité native E2 (ADR-0035 §6) : `visible`, `enabled`
//! et `topmost` lus en un round-trip de sondes
//! (`geometry_report` + `element_info` + `hit_covers`), `stable`
//! constaté par deux lectures de rect consécutives identiques
//! dans la boucle d'auto-wait. Le polling est borné et honnête
//! — les hooks de commit (E2+ côté core) remplaceront la boucle
//! par de la souscription sans changer le contrat.

use std::sync::Arc;
use std::time::Duration;

use serde_json::json;

use crate::proto::{ActionOpts, Actionability, Check, ErrorCode, Rect, RpcError};
use crate::state::{AgentConn, AgentShared, Handle};

/// Budget d'auto-wait par défaut (règle 13 — nommé).
const DEFAULT_WAIT_MS: u64 = 5_000;

/// Cadence de la boucle d'actionabilité — assez basse pour
/// suivre une animation, assez haute pour ne pas verrouiller
/// la session en continu.
pub(crate) const POLL_MS: u64 = 50;

/// Constat d'une lecture d'élément — rect **document** (px
/// CSS), attributs, couverture hit-test.
#[derive(Debug, Clone)]
pub struct Probe {
    /// Boîte de l'élément dans le rapport géométrique — `None`
    /// = non posé (display:none, hors arbre de layout).
    pub rect: Option<Rect>,
    /// `disabled` ou `aria-disabled="true"` sur l'élément.
    pub disabled: bool,
    /// Le point central de la boîte atteint l'élément ou un
    /// descendant (`hit_covers` natif) — `None` = indéterminable.
    pub topmost: Option<bool>,
    /// Tag minuscule (`button`, `input`…).
    pub tag: String,
    /// Attributs source — `type`, `href`, `aria-label`…
    pub attrs: Vec<(String, String)>,
}

impl Probe {
    /// Attribut source.
    #[must_use]
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Centre de la boîte en coordonnées **viewport** (le
    /// scroll est retranché — `hit_covers`/`input` raisonnent
    /// en px CSS viewport).
    #[must_use]
    pub fn center_viewport(&self, scroll_y: f32) -> Option<(f32, f32)> {
        self.rect
            .map(|r| (r.x + r.w / 2.0, r.y - scroll_y + r.h / 2.0))
    }
}

/// Résout la ref en handle vivant — `StaleRef` si inconnue,
/// mauvais onglet ou document re-commit depuis l'émission.
///
/// # Errors
///
/// [`ErrorCode::StaleRef`] — ref inconnue, autre onglet, ou
/// `rev` divergeant (re-snapshoter côté agent).
pub fn resolve(
    conn: &AgentConn,
    st: &Arc<AgentShared>,
    tab: u64,
    elem_ref: &str,
) -> Result<Handle, RpcError> {
    conn.handles
        .resolve(elem_ref, tab, st.rev(tab))
        .map_err(|code| RpcError {
            code: code.into(),
            message: format!("ref {elem_ref} expirée — re-snapshoter"),
            data: Some(json!({ "ref": elem_ref })),
        })
}

/// Une lecture complète de l'élément — `StaleRef` si le nœud
/// est mort entre l'émission de la ref et maintenant (la
/// génération est sonde-fraîche, pas celle figée à l'alloc :
/// un nœud réattribué est détecté ici, jamais résolu à tort).
///
/// # Errors
///
/// [`ErrorCode::StaleRef`] — nœud mort ou réattribué.
pub async fn probe(st: &Arc<AgentShared>, tab: u64, index: u32) -> Result<Probe, RpcError> {
    let s = st.session.lock().await;
    let Some(generation) = s.node_generation(tab, index) else {
        return Err(RpcError {
            code: ErrorCode::StaleRef.into(),
            message: "nœud mort ou réattribué".into(),
            data: None,
        });
    };
    let scroll_y = s.tab_info(tab).map_or(0.0, |t| t.scroll_y);
    let report = s.geometry_report(tab);
    let rect = report.and_then(|r| {
        r.boxes.iter().find(|b| b.node == index).map(|b| Rect {
            x: b.x,
            y: b.y,
            w: b.w,
            h: b.h,
        })
    });
    let info = s.element_info(tab, index, generation);
    let (tag, attrs, disabled) = match &info {
        Some(i) => (
            i.tag.clone(),
            i.attrs.clone(),
            i.attr("disabled").is_some() || i.attr("aria-disabled") == Some("true"),
        ),
        None => (String::new(), Vec::new(), false),
    };
    let topmost = rect.and_then(|r| {
        let (cx, cy) = (r.x + r.w / 2.0, r.y - scroll_y + r.h / 2.0);
        s.hit_covers(tab, cx, cy, index, generation)
    });
    Ok(Probe {
        rect,
        disabled,
        topmost,
        tag,
        attrs,
    })
}

/// Les checks requis évalués sur un probe — `stable` est pris
/// en argument (constat inter-lectures, pas observable sur une
/// lecture unique).
fn evaluate(p: &Probe, stable: bool, vh: f32, scroll_y: f32) -> Actionability {
    let visible = p.rect.is_some_and(|r| {
        r.w > 0.0 && r.h > 0.0 && r.y - scroll_y + r.h > 0.0 && r.y - scroll_y < vh
    });
    Actionability {
        visible,
        stable,
        enabled: !p.disabled,
        topmost: p.topmost.unwrap_or(false),
    }
}

/// `true` si tous les checks `require` sont vrais dans `a`.
fn passes(a: Actionability, require: &[Check]) -> bool {
    require.iter().all(|c| match c {
        Check::Visible => a.visible,
        Check::Stable => a.stable,
        Check::Enabled => a.enabled,
        Check::Topmost => a.topmost,
    })
}

/// Auto-wait borné : sonde → évalue → retour dès que `require`
/// passe, sinon nouvelle lecture toutes les `POLL_MS` jusqu'à
/// `opts.timeout_ms`. `stable` demande deux lectures de rect
/// identiques consécutives. Au délai échu : `opts.force` agit
/// au dernier probe (résultat marqué `degraded`), sinon
/// [`ErrorCode::NotActionable`] avec le constat en `data`.
///
/// # Errors
///
/// [`ErrorCode::NotActionable`] — checks requis jamais
/// satisfaits dans le budget (constat en `data.actionability`) ;
/// [`ErrorCode::StaleRef`] — nœud mort en cours d'attente.
pub async fn auto_wait(
    st: &Arc<AgentShared>,
    tab: u64,
    handle: Handle,
    opts: &ActionOpts,
) -> Result<(Probe, Actionability, bool), RpcError> {
    let require: Vec<Check> = opts.require.clone().unwrap_or_else(|| {
        vec![
            Check::Visible,
            Check::Stable,
            Check::Enabled,
            Check::Topmost,
        ]
    });
    let budget = Duration::from_millis(opts.timeout_ms.map_or(DEFAULT_WAIT_MS, u64::from));
    let deadline = tokio::time::Instant::now() + budget;
    let mut prev_rect = None::<Rect>;

    loop {
        let p = probe(st, tab, handle.node).await?;
        let vh = st.session.lock().await.viewport().1;
        let scroll_y = st
            .session
            .lock()
            .await
            .tab_info(tab)
            .map_or(0.0, |t| t.scroll_y);
        // `stable` = deux lectures consécutives rendent le même
        // rect — la première lecture n'est jamais stable.
        let stable = prev_rect == p.rect && p.rect.is_some();
        prev_rect = p.rect;
        let action = evaluate(&p, stable, vh, scroll_y);
        if passes(action, &require) {
            return Ok((p, action, false));
        }
        if tokio::time::Instant::now() >= deadline {
            if opts.force {
                return Ok((p, action, true));
            }
            return Err(RpcError {
                code: ErrorCode::NotActionable.into(),
                message: "checks d'actionabilité échus".into(),
                data: Some(json!({ "actionability": action })),
            });
        }
        tokio::time::sleep(Duration::from_millis(POLL_MS)).await;
    }
}
