// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Sondes de session pour la surface agent EAP (ADR-0035,
//! Étape 244) — lectures **additives** du DOM/layout commis :
//! sélection CSS native, hit-test par cible, attributs
//! d'élément. Rien ici ne mute la session ; tout réutilise les
//! primitives publiques des couches internes
//! (`explorer_css::matches`, `hit_test`, `dom.attributes`).

use explorer_dom::{Attribute, NodeId};

use super::dom_helpers::hit_test;
use super::{CoreError, Session};
use crate::session::types::TabId;

/// Attributs d'un élément sérialisés pour l'agent — paires
/// `(nom, valeur)` dans l'ordre source.
#[derive(Debug, Clone)]
pub struct ElementInfo {
    /// Nom de balise minuscule.
    pub tag: String,
    /// Attributs source.
    pub attrs: Vec<(String, String)>,
}

impl ElementInfo {
    /// Valeur d'un attribut (`None` si absent).
    #[must_use]
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Ligne sémantique d'un élément — une entrée de la marche
/// [`Session::snapshot_entries`], jointe à la géométrie par
/// `index` côté agent.
#[derive(Debug, Clone)]
pub struct SnapshotEntry {
    /// Index arène du nœud (clé de jointure `GeometryReport`).
    pub index: u32,
    /// Génération à l'émission — figée dans les handles.
    pub generation: u32,
    /// Nom de balise minuscule.
    pub tag: String,
    /// Attributs source (rôle ARIA, `type`, `disabled`…).
    pub attrs: Vec<(String, String)>,
    /// Profondeur réelle dans le DOM (0 = racine document).
    pub depth: u16,
    /// Texte visible compacté, plafonné (nom accessible).
    pub text: String,
    /// `document.activeElement` === ce nœud.
    pub focused: bool,
}

impl SnapshotEntry {
    /// Valeur d'un attribut (`None` si absent).
    #[must_use]
    pub fn attr(&self, name: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

impl Session {
    /// Sélection CSS **native** sur le DOM commis de l'onglet :
    /// le prélude est parsé par le moteur `explorer-css` (toute
    /// la grammaire supportée, pas un sous-ensemble) puis
    /// `matches` marche les descendants du document en pré-ordre.
    /// Retourne les index d'arène des éléments matchants, dans
    /// l'ordre document — l'agent les fige en handles
    /// `(index, génération)` au moment de l'émission.
    ///
    /// `Err(Selector)` si le prélude n'est pas un sélecteur
    /// valide (dégradation spec : la règle est rejetée, pas
    /// l'agent) ; `Err(UnknownTab)` si l'onglet n'existe pas ;
    /// `Err(EmptyDocument)` si le DOM est détaché (apply en vol
    /// — l'agent réessaie).
    ///
    /// # Errors
    ///
    /// [`CoreError::Selector`], [`CoreError::UnknownTab`],
    /// [`CoreError::EmptyDocument`].
    pub fn select_nodes(&self, tab: TabId, selector: &str) -> Result<Vec<u32>, CoreError> {
        let selectors = explorer_css::parse_selector_list(selector)
            .ok_or_else(|| CoreError::Selector(selector.to_string()))?;
        let entry = self.tabs.get(&tab).ok_or(CoreError::UnknownTab(tab))?;
        let dom = entry.dom.as_ref().ok_or(CoreError::EmptyDocument)?;
        let mut out = Vec::new();
        for node in dom
            .descendants(dom.document())
            .map_err(|_| CoreError::EmptyDocument)?
        {
            if dom.element(node).is_ok() && explorer_css::matches_any(dom, node, &selectors) {
                out.push(node.index());
            }
        }
        Ok(out)
    }

    /// La génération courante du nœud d'index `index` —
    /// `None` si l'index ne désigne plus un élément vivant
    /// (handle expiré : mutation ou re-parse, jamais de
    /// résolution silencieuse sur une case réattribuée).
    #[must_use]
    pub fn node_generation(&self, tab: TabId, index: u32) -> Option<u32> {
        let dom = self.tabs.get(&tab)?.dom.as_ref()?;
        let node = Self::find_node(dom, index)?;
        Some(node.generation())
    }

    /// Les attributs de l'élément `(index, generation)` — la
    /// paire index+génération garantit que le handle cible bien
    /// le nœud vu à l'émission (une case réattribuée porte une
    /// génération différente → `None` → `stale_ref` côté agent).
    /// Sert aux checks `enabled` (`disabled`/`aria-disabled`),
    /// aux noms accessibles (`aria-label`/`alt`/`title`) et aux
    /// états de forme (`type`, `checked`, `value`).
    #[must_use]
    pub fn element_info(&self, tab: TabId, index: u32, generation: u32) -> Option<ElementInfo> {
        let dom = self.tabs.get(&tab)?.dom.as_ref()?;
        let node = Self::find_node(dom, index)?;
        if node.generation() != generation {
            return None;
        }
        let attrs: Vec<(String, String)> = dom
            .attributes(node)
            .map(|attrs: &[Attribute]| {
                attrs
                    .iter()
                    .map(|a| (a.name.to_string(), a.value.clone()))
                    .collect()
            })
            .unwrap_or_default();
        Some(ElementInfo {
            tag: dom.tag_name(node).unwrap_or("#text").to_string(),
            attrs,
        })
    }

    /// Le point viewport `(x, y)` atteint-il l'élément
    /// `(index, generation)` ou l'un de ses descendants ?
    /// (check `topmost` : un `<span>` dans un `<a>` cliqué au
    /// centre de l'ancre rend `true` — le hit-test descend sur
    /// le texte, la couverture remonte aux ancêtres.)
    /// `None` si l'onglet/l'arbre est indisponible ou si la
    /// génération ne correspond plus.
    #[must_use]
    pub fn hit_covers(
        &self,
        tab: TabId,
        x: f32,
        y: f32,
        index: u32,
        generation: u32,
    ) -> Option<bool> {
        let entry = self.tabs.get(&tab)?;
        let (tree, dom) = (entry.tree.as_ref()?, entry.dom.as_ref()?);
        let target = Self::find_node(dom, index)?;
        if target.generation() != generation {
            return None;
        }
        let hit = hit_test(tree, x, y, entry.scroll_y, self.viewport)?;
        // Le nœud touché couvre la cible s'il en descend —
        // remonter les ancêtres jusqu'à la racine document.
        let mut cur = Some(hit);
        while let Some(n) = cur {
            if n == target {
                return Some(true);
            }
            cur = dom.parent(n).ok().flatten();
        }
        Some(false)
    }

    /// Marche sémantique du document commis — une passe
    /// pré-ordre qui collecte, pour chaque **élément**, la
    /// génération, le tag, les attributs, la profondeur réelle
    /// et le texte visible compacté. Sert au snapshot E3
    /// (rôle/nom/états côté agent) : la géométrie est jointe
    /// par index arène au `geometry_report` — cette sonde ne
    /// touche pas au box tree.
    ///
    /// Bornes (contenu hostile) : `SNAP_CAP` éléments maximum,
    /// texte compacté plafonné à `TEXT_CAP` caractères par
    /// nœud, profondeur mémoïsée en une passe (les parents
    /// précèdent toujours en pré-ordre).
    ///
    /// # Errors
    ///
    /// [`CoreError::UnknownTab`], [`CoreError::EmptyDocument`].
    pub fn snapshot_entries(&self, tab: TabId) -> Result<Vec<SnapshotEntry>, CoreError> {
        const SNAP_CAP: usize = 8192;
        const TEXT_CAP: usize = 160;
        let entry = self.tabs.get(&tab).ok_or(CoreError::UnknownTab(tab))?;
        let dom = entry.dom.as_ref().ok_or(CoreError::EmptyDocument)?;
        // Focus résolu en lecture seule — même vérification
        // d'ancre `id`/`name` que `focused_node` (étape 92),
        // mais sans ré-ancrage : une sonde ne mute jamais.
        let focused = entry.focused.as_deref().and_then(|path| {
            let n = super::dom_helpers::dom_path_node(dom, path)?;
            let ok = entry
                .focused_key
                .as_ref()
                .is_none_or(|(a, v)| dom.attribute_str(n, a).ok().flatten() == Some(v.as_str()));
            ok.then_some(n)
        });
        let mut depths: std::collections::HashMap<NodeId, u16> = std::collections::HashMap::new();
        let mut out = Vec::new();
        for node in dom
            .descendants(dom.document())
            .map_err(|_| CoreError::EmptyDocument)?
        {
            if out.len() >= SNAP_CAP {
                break;
            }
            if dom.element(node).is_err() {
                continue;
            }
            let depth = match dom.parent(node).ok().flatten() {
                Some(p) => depths.get(&p).copied().unwrap_or(0).saturating_add(1),
                None => 0,
            };
            depths.insert(node, depth);
            let attrs: Vec<(String, String)> = dom
                .attributes(node)
                .map(|attrs: &[Attribute]| {
                    attrs
                        .iter()
                        .map(|a| (a.name.to_string(), a.value.clone()))
                        .collect()
                })
                .unwrap_or_default();
            // Texte propre borné : whitespace compacté + cap —
            // le nom accessible n'a besoin que du début.
            let text = dom
                .text_content(node)
                .map(|t| {
                    let collapsed: String = t.split_whitespace().collect::<Vec<_>>().join(" ");
                    collapsed.chars().take(TEXT_CAP).collect()
                })
                .unwrap_or_default();
            out.push(SnapshotEntry {
                index: node.index(),
                generation: node.generation(),
                tag: dom.tag_name(node).unwrap_or("?").to_string(),
                attrs,
                depth,
                text,
                focused: focused == Some(node),
            });
        }
        Ok(out)
    }

    /// Résout un index d'arène en `NodeId` vivant — scan des
    /// descendants du document (O(n), chemin non chaud : appels
    /// agent bornés par l'actionabilité, pas par frame).
    fn find_node(dom: &explorer_dom::Dom, index: u32) -> Option<NodeId> {
        dom.descendants(dom.document())
            .ok()?
            .find(|n| n.index() == index)
    }
}
