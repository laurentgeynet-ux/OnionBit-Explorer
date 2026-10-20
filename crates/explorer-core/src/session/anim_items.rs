// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Items d'animation : transitions CSS et `@keyframes` cadencés
//! (R-10) — synchronisation de style et état courant.
use std::collections::HashMap;
use std::time::{Duration, Instant};

use explorer_css::animation::{animation_decls, keyframe_style, AnimationDecl};
use explorer_css::transition::{interpolate, transition_decls, Timing};
use explorer_css::{initial_value, ComputedStyle, StyleEngine, StyleMap};
use explorer_dom::{Dom, NodeId};

/// Transition en vol sur `(nœud, propriété)` — étape 45 :
/// `from`/`to` sont les valeurs calculées textuelles
/// (interpolées par `explorer_css::transition`), `start`
/// l'`Instant` d'enregistrement.
#[derive(Clone)]
pub(crate) struct ActiveTransition {
    /// Ordinal de l'élément en ordre document — stable au
    /// re-parse (`NodeId` régénéré), seules les insertions
    /// structurelles décalent (déviation MVP documentée).
    pub(crate) elem: usize,
    pub(crate) prop: String,
    pub(crate) from: String,
    pub(crate) to: String,
    pub(crate) start: Instant,
    /// Durée en secondes (> 0 garanti à l'enregistrement).
    pub(crate) duration: f64,
    /// Délai en secondes (négatif = déjà entamée, spec).
    pub(crate) delay: f64,
    pub(crate) timing: Timing,
}

impl ActiveTransition {
    /// Instant de fin (`start + max(0, delay + duration)`).
    pub(crate) fn end(&self) -> Instant {
        self.start + Duration::from_secs_f64((self.delay + self.duration).max(0.0))
    }

    /// Valeur affichée à `now` : `Some` tant que la
    /// transition n'est pas terminée — la phase de délai
    /// reste sur `from`, une valeur non interpolable commute
    /// discrètement à `to` en fin (spec).
    pub(crate) fn value_at(&self, now: Instant) -> Option<String> {
        let elapsed = now.saturating_duration_since(self.start).as_secs_f64() - self.delay;
        if elapsed >= self.duration {
            return None;
        }
        let p = self.timing.apply(elapsed / self.duration);
        Some(interpolate(&self.from, &self.to, p).unwrap_or_else(|| {
            if p < 1.0 {
                self.from.clone()
            } else {
                self.to.clone()
            }
        }))
    }
}

/// Animation `@keyframes` enregistrée sur `(élément, index
/// de la liste `animation`)` — étape 55e. `start` persiste
/// tant que la déclaration est inchangée : un re-style
/// (tick, mutation) ne redémarre pas la lecture.
#[derive(Clone)]
pub(crate) struct ActiveAnimation {
    /// Ordinal d'élément en ordre document (même convention
    /// que [`ActiveTransition`]).
    pub(crate) elem: usize,
    /// Index dans `animation_decls` — les items s'alignent
    /// par position (un `none` en tête décale les suivants).
    pub(crate) idx: usize,
    /// Déclaration résolue à l'enregistrement.
    pub(crate) decl: AnimationDecl,
    pub(crate) start: Instant,
    /// La dernière passe a écrit des surcharges — tant que
    /// c'est vrai, une lecture qui vient de se terminer sans
    /// `fill` doit encore produire **un** reflow pour
    /// replier le style de base (sinon la frame reste figée
    /// sur la valeur animée).
    pub(crate) applied: bool,
}

impl ActiveAnimation {
    /// Lecture digne d'un tick à `now` — `true` tant que la
    /// fenêtre de lecture court (délai inclus : l'animation
    /// doit démarrer à son terme sans déclencheur externe)
    /// ou qu'un repli final reste à peindre (`applied` +
    /// phase désormais `None`). Une animation terminée en
    /// `forwards` ne retourne pas `true` : sa valeur figée
    /// est ré-appliquée à chaque passe, sans reflow.
    pub(crate) fn playing(&self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.start).as_secs_f64();
        elapsed - self.decl.delay < self.decl.iterations * self.decl.duration
            || (self.applied && self.decl.phase(elapsed).is_none())
    }
}

/// Identité de déclaration — un re-style identique ne
/// redémarre pas l'animation (champ par champ :
/// `AnimationDecl` ne dérive pas `PartialEq` côté PIPE).
pub(crate) fn same_decl(a: &AnimationDecl, b: &AnimationDecl) -> bool {
    a.name == b.name
        && a.duration.to_bits() == b.duration.to_bits()
        && a.delay.to_bits() == b.delay.to_bits()
        && a.timing == b.timing
        && a.iterations.to_bits() == b.iterations.to_bits()
        && a.direction == b.direction
        && a.fill == b.fill
}

/// Borne anti-hostile : transitions simultanées par onglet
/// (une page hostile peut déclencher des milliers de diffs).
pub(crate) const MAX_TRANSITIONS: usize = 256;

/// `NodeId` → ordinal d'élément en ordre document (étape
/// 45) — clé stable au re-parse des synchronisations DOM :
/// les attributs mutés ne changent pas l'ordre, seules les
/// insertions d'éléments décalent les suivants (leurs
/// transitions repartent alors — déviation MVP).
pub(crate) fn element_ordinals(dom: &Dom) -> HashMap<NodeId, usize> {
    let mut out = HashMap::new();
    let mut queue = vec![dom.document()];
    while let Some(n) = queue.pop() {
        if dom.element(n).is_ok() {
            out.insert(n, out.len());
        }
        if let Ok(ch) = dom.children(n) {
            queue.extend(ch.iter().rev());
        }
    }
    out
}

/// Diff `prev_styles` → `styles` (étape 45) : enregistre les
/// nouvelles transitions (`transition-*` déclarés, valeur
/// modifiée) et retarget les transitions en vol depuis leur
/// valeur courante. Renvoie `true` s'il reste des
/// transitions actives. `prev` et `styles` DOIVENT être les
/// maps bruts (sans les surcharges d'animation — appliquées
/// plus tard par [`apply_anim_overrides`], étape 55e),
/// indexés par ordinal d'élément.
pub(crate) fn update_transitions(
    prev: &HashMap<usize, ComputedStyle>,
    transitions: &mut Vec<ActiveTransition>,
    styles: &StyleMap,
    ord_of: &HashMap<NodeId, usize>,
    now: Instant,
) -> bool {
    let mut fresh: Vec<ActiveTransition> = Vec::new();
    for (node, cs) in styles.iter() {
        let Some(&ord) = ord_of.get(&node) else {
            continue;
        };
        // Pas de « before-change style » (premier rendu,
        // nœud inséré) → pas de transition (spec).
        let Some(old) = prev.get(&ord) else {
            continue;
        };
        for decl in transition_decls(cs)
            .iter()
            .filter(|d| d.duration > 0.0 && d.property != "none")
        {
            let targets: Vec<String> = if decl.property == "all" {
                cs.iter()
                    .map(|(k, _)| k.to_string())
                    .filter(|k| !k.starts_with("transition"))
                    .collect()
            } else {
                vec![decl.property.clone()]
            };
            for prop in targets {
                let new = cs
                    .get(&prop)
                    .or_else(|| initial_value(&prop))
                    .unwrap_or("")
                    .to_string();
                let ongoing = transitions
                    .iter()
                    .position(|t| t.elem == ord && t.prop == prop);
                if let Some(i) = ongoing {
                    if transitions[i].to == new {
                        // Même cible — la transition continue.
                        fresh.push(transitions[i].clone());
                        continue;
                    }
                }
                let oldv = old
                    .get(&prop)
                    .or_else(|| initial_value(&prop))
                    .unwrap_or("")
                    .to_string();
                if new == oldv && ongoing.is_none() {
                    continue;
                }
                // Retarget : repart de la valeur actuellement
                // affichée, sinon de l'ancienne calculée.
                let from = ongoing
                    .and_then(|i| transitions[i].value_at(now))
                    .unwrap_or(oldv);
                fresh.retain(|t| !(t.elem == ord && t.prop == prop));
                if from != new {
                    fresh.push(ActiveTransition {
                        elem: ord,
                        prop,
                        from,
                        to: new,
                        start: now,
                        duration: decl.duration,
                        delay: decl.delay,
                        timing: decl.timing,
                    });
                }
            }
        }
    }
    // Transitions non revisitées : survivent si leur
    // déclaration existe encore, la cible reste la valeur
    // calculée et elles ne sont pas finies.
    let cs_of: HashMap<usize, &ComputedStyle> = styles
        .iter()
        .filter_map(|(n, c)| ord_of.get(&n).map(|o| (*o, c)))
        .collect();
    for t in transitions.drain(..) {
        if fresh.iter().any(|f| f.elem == t.elem && f.prop == t.prop) || t.end() <= now {
            continue;
        }
        let alive = cs_of.get(&t.elem).is_some_and(|c| {
            c.get(&t.prop)
                .or_else(|| initial_value(&t.prop))
                .unwrap_or("")
                == t.to
                && transition_decls(c)
                    .iter()
                    .any(|d| d.duration > 0.0 && (d.property == "all" || d.property == t.prop))
        });
        if alive {
            fresh.push(t);
        }
    }
    transitions.clone_from(&fresh);
    transitions.truncate(MAX_TRANSITIONS);
    // Les surcharges ne sont PAS appliquées ici : les
    // `@keyframes` passent sous les transitions (couche
    // « current transition » du spec) — séquencement dans
    // `apply_anim_overrides` (étape 55e).
    let running = transitions.iter().any(|t| t.end() > now);
    transitions.retain(|t| t.end() > now);
    running
}

/// Enregistre/renouvelle les animations `@keyframes` de la
/// passe de styles brute (étape 55e) : identité
/// `(élément, index)` + déclaration — une déclaration
/// inchangée conserve son `start` (le tick ne redémarre
/// pas), changée elle repart de `now`. Les éléments dont la
/// déclaration disparaît perdent leur animation.
pub(crate) fn sync_animations(
    animations: &mut Vec<ActiveAnimation>,
    styles: &StyleMap,
    ord_of: &HashMap<NodeId, usize>,
    eng: &StyleEngine,
    now: Instant,
) {
    let mut fresh = Vec::with_capacity(animations.len());
    for (node, cs) in styles.iter() {
        let Some(&ord) = ord_of.get(&node) else {
            continue;
        };
        for (idx, decl) in animation_decls(cs).iter().enumerate() {
            if decl.name == "none" || eng.keyframes(&decl.name).is_none() {
                continue;
            }
            let same = animations
                .iter()
                .find(|a| a.elem == ord && a.idx == idx && same_decl(&a.decl, decl));
            fresh.push(match same {
                Some(a) => a.clone(),
                None => ActiveAnimation {
                    elem: ord,
                    idx,
                    decl: decl.clone(),
                    start: now,
                    applied: false,
                },
            });
        }
    }
    *animations = fresh;
    animations.truncate(MAX_TRANSITIONS);
}

/// Surcharges d'animation sur `styles` (post-cascade —
/// étape 55e) : les `@keyframes` d'abord (couche
/// animation), les transitions ensuite (couche « current
/// transition » du spec — elles l'emportent). Une frame
/// absente d'une borne part de la valeur calculée via
/// `keyframe_style` ; une phase `None` (avant délai sans
/// `backwards`, terminé sans `forwards`) n'écrit rien.
pub(crate) fn apply_anim_overrides(
    transitions: &[ActiveTransition],
    animations: &mut [ActiveAnimation],
    eng: &StyleEngine,
    styles: &mut StyleMap,
    ord_of: &HashMap<NodeId, usize>,
    now: Instant,
) {
    let node_of: HashMap<usize, NodeId> = ord_of.iter().map(|(n, o)| (*o, *n)).collect();
    for a in animations.iter_mut() {
        a.applied = false;
        let Some(frames) = eng.keyframes(&a.decl.name) else {
            continue;
        };
        let elapsed = now.saturating_duration_since(a.start).as_secs_f64();
        let Some(phase) = a.decl.phase(elapsed) else {
            continue;
        };
        let Some(cs) = node_of.get(&a.elem).and_then(|n| styles.get_mut(*n)) else {
            continue;
        };
        for (k, v) in keyframe_style(frames, phase, a.decl.timing, cs) {
            cs.set(&k, &v);
        }
        a.applied = true;
    }
    for t in transitions {
        if let Some(v) = t.value_at(now) {
            if let Some(cs) = node_of.get(&t.elem).and_then(|n| styles.get_mut(*n)) {
                cs.set(&t.prop, &v);
            }
        }
    }
}
