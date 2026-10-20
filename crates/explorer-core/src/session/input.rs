// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Entrées : hit-test, clics, clavier, focus, popup
//! `<select>`, soumission de formulaires, scroll.
use super::*;

use explorer_dom::NodeId;
use explorer_net::http::Request;
use explorer_net::url::Url;

impl Session {
    /// Applique un événement d'entrée à l'onglet : hit-test
    /// sur l'arbre posé, scroll borné, suivi de lien.
    /// Corps synchrone + drain compat `&mut self` (étape
    /// 53d) — les hôtes à session partagée appellent
    /// [`Self::handle_input_now`] puis `drain_navs_shared`.
    ///
    /// # Errors
    /// [`CoreError::UnknownTab`], ou erreur de navigation si
    /// un clic de lien déclenche un fetch qui échoue.
    pub async fn handle_input(
        &mut self,
        tab: TabId,
        ev: InputEvent,
    ) -> Result<InputOutcome, CoreError> {
        let out = self.handle_input_now(tab, ev)?;
        self.drain_script_nav(tab).await;
        Ok(out)
    }

    /// Corps synchrone de [`Self::handle_input`] (étape
    /// 53d) : dispatche l'événement DOM et pose les
    /// intentions de navigation dans `pending_nav` —
    /// **jamais de fetch sous `&mut self`**. Les hôtes à
    /// session partagée enchaînent `drain_navs_shared` ;
    /// `handle_input` draine en compat.
    ///
    /// # Errors
    /// [`CoreError::UnknownTab`] ou erreur de pipeline
    /// d'édition/re-layout.
    pub fn handle_input_now(
        &mut self,
        tab: TabId,
        ev: InputEvent,
    ) -> Result<InputOutcome, CoreError> {
        if !self.tabs.contains_key(&tab) {
            return Err(CoreError::UnknownTab(tab));
        }
        // Un input sur une page « commise mais jamais peinte »
        // (56b) repart d'une frame nulle : hit-test mort
        // sinon — auto-réparation avant tout dispatch.
        self.ensure_painted(tab);
        match ev {
            InputEvent::Key { key } => {
                // `keydown` sur le champ focalisé, sinon niveau
                // document (étape 52) — `preventDefault`
                // supprime l'édition et la soumission `Enter`.
                if self.dispatch_key(tab, "keydown") {
                    return Ok(InputOutcome::Ignored);
                }
                // Clavier du popup `<select>` ouvert : les
                // flèches déplacent le surlignage, `Enter`
                // choisit, `Escape` referme — consommé avant
                // tout focus d'édition (étape 54).
                if self.tabs.get(&tab).is_some_and(|e| e.popup.is_some()) {
                    return self.popup_key(tab, &key);
                }
                // Soumission implicite : `Enter` dans un champ
                // d'un `<form>` suit la séquence `try_submit`
                // complète — validation + `submit` + bouton
                // par défaut comme `submitter` (HTML).
                if key == "Enter" {
                    if let Some((form, sub)) = self.focused_form_default(tab) {
                        self.set_focus(tab, None);
                        return self.try_submit(tab, form, sub, false);
                    }
                }
                self.handle_key(tab, &key)
            }
            InputEvent::KeyUp { key: _ } => {
                self.dispatch_key(tab, "keyup");
                Ok(InputOutcome::Ignored)
            }
            InputEvent::Scroll { dy } => {
                // `wheel` (annulable) sur le nœud survolé ou le
                // document — `preventDefault` bloque le scroll.
                // Un popup `<select>` ouvert se referme au
                // scroll (étape 54 — ancrage document).
                if let Some(entry) = self.tabs.get_mut(&tab) {
                    entry.popup = None;
                }
                if !self.dispatch_wheel(tab) {
                    self.scroll_by(tab, dy)?;
                    // `scroll` niveau document (non annulable).
                    self.dispatch_doc_event(tab, "scroll");
                }
                Ok(InputOutcome::Scrolled {
                    scroll_y: self.tabs.get(&tab).map_or(0.0, |t| t.scroll_y),
                })
            }
            InputEvent::MouseMove { x, y } => self.handle_mouse_move(tab, x, y),
            InputEvent::Click { x, y } => self.handle_click(tab, x, y),
        }
    }

    /// Survol suivi (étape 52) : `mouseout` sur l'ancienne
    /// cible, `mouseover`/`mousemove` sur la nouvelle — déjà
    /// coalescé côté client (ADR-0003), un événement par
    /// `move`. Les dispatches se font par **chemin** : chaque
    /// handler peut muter le miroir et re-parser le document —
    /// un `NodeId` capté en début de bras serait mort dès le
    /// premier dispatch.
    pub(crate) fn handle_mouse_move(
        &mut self,
        tab: TabId,
        x: f32,
        y: f32,
    ) -> Result<InputOutcome, CoreError> {
        // Popup `<select>` ouvert : le curseur pilote le
        // surlignage de ligne — capture chrome, pas
        // d'événements DOM vers le document (étape 54).
        let pop_hit = self.tabs.get_mut(&tab).and_then(|entry| {
            entry.popup.as_mut().map(|pop| {
                let row = popup_row(pop, x, y + entry.scroll_y, self.viewport.0, entry.doc_h);
                let changed = pop.hovered != row;
                pop.hovered = row;
                changed
            })
        });
        if let Some(changed) = pop_hit {
            if changed {
                self.repaint(tab)?;
                self.emit_invalidated(tab);
            }
            return Ok(InputOutcome::Hover {
                url: None,
                editable: false,
            });
        }
        let node = self.node_at(tab, x, y);
        let path = node.and_then(|n| {
            self.tabs
                .get(&tab)
                .and_then(|e| e.dom.as_ref())
                .and_then(|d| element_path(d, n))
        });
        let prev = self.tabs.get(&tab).and_then(|t| t.hovered.clone());
        if prev != path {
            if let Some(o) = prev {
                self.dispatch_path_event(tab, "mouseout", &o);
            }
            if let Some(p) = &path {
                self.dispatch_path_event(tab, "mouseover", p);
            }
            if let Some(entry) = self.tabs.get_mut(&tab) {
                entry.hovered.clone_from(&path);
            }
        }
        if let Some(p) = &path {
            self.dispatch_path_event(tab, "mousemove", p);
        }
        self.collect_script_navs(tab);
        if self.nav_pending(tab) {
            return Ok(InputOutcome::Navigated);
        }
        Ok(InputOutcome::Hover {
            url: self.link_at(tab, x, y),
            editable: self.field_at(tab, x, y).is_some(),
        })
    }

    /// Clic : `click` DOM d'abord (`preventDefault` annule
    /// l'action par défaut), bascule des contrôles puis
    /// `input`/`change`, focus sur les champs, `submit` des
    /// formulaires, navigation des liens.
    pub(crate) fn handle_click(
        &mut self,
        tab: TabId,
        x: f32,
        y: f32,
    ) -> Result<InputOutcome, CoreError> {
        // Popup `<select>` ouvert : le clic est consommé par
        // l'overlay chrome — choix d'option ou fermeture,
        // sans `click` DOM sur le document (étape 54).
        if let Some(out) = self.popup_click(tab, x, y)? {
            return Ok(out);
        }
        let node = self.node_at(tab, x, y);
        // Contrôle à bascule : activation AVANT `click`
        // (spec DOM — le handler lit `checked` post-bascule
        // via le miroir synchronisé) ; `preventDefault`
        // annule et restaure l'état.
        let toggled = self.toggle_at(tab, x, y);
        // Chemin du contrôle capté AVANT le `click` : le
        // handler peut re-parser le document — le `NodeId`
        // meurt, le chemin survit.
        let ctrl_path = toggled.and_then(|c| {
            self.tabs
                .get(&tab)
                .and_then(|e| e.dom.as_ref())
                .and_then(|d| element_path(d, c))
        });
        if let Some(ctrl) = toggled {
            self.flip_checked(tab, ctrl)?;
        }
        // L'événement DOM précède l'action par défaut : un
        // handler peut muter la page (resynchronisée et
        // re-peinte ici) ou `preventDefault`.
        let prevented = node.is_some_and(|n| self.dispatch_dom_event(tab, "click", n));
        if toggled.is_some() {
            // Re-résolution sur l'arbre courant pour le revert
            // ou la suite `input`/`change`.
            let ctrl = self.resolve_path(tab, ctrl_path.as_deref());
            if prevented {
                if let Some(c) = ctrl {
                    self.flip_checked(tab, c)?;
                }
            } else if let Some(p) = &ctrl_path {
                // `input` puis `change` — ordre DOM.
                self.dispatch_path_event(tab, "input", p);
                self.dispatch_path_event(tab, "change", p);
            }
            self.relayout(tab)?;
            self.emit_invalidated(tab);
            return Ok(if prevented {
                InputOutcome::Ignored
            } else {
                InputOutcome::Edited
            });
        }
        // Navigation script (`location`, `history`) : priorité
        // sur l'action par défaut du clic.
        self.collect_script_navs(tab);
        if self.nav_pending(tab) {
            return Ok(InputOutcome::Navigated);
        }
        if let Some(field) = self.field_at(tab, x, y) {
            // `blur` sur l'ancien focus puis `focus` sur le
            // champ (étape 52).
            self.set_focus(tab, Some(field));
            return Ok(InputOutcome::Focused);
        }
        self.set_focus(tab, None);
        // `<select>` : ouvre/ferme le popup options moteur
        // (étape 54 — overlay de la display list, peint
        // chrome-side, remplace le cycle MVP d'ADR-0014).
        if let Some(sel) = self.select_at(tab, x, y) {
            self.toggle_select_popup(tab, sel)?;
            self.emit_invalidated(tab);
            return Ok(InputOutcome::Edited);
        }
        // Contrôle submit : validation de contraintes puis
        // événement `submit` puis action par défaut — la
        // séquence est factorisée dans `try_submit` (partagée
        // avec la soumission implicite `Enter`).
        if let Some((form, sub)) = self.submit_at(tab, x, y) {
            return self.try_submit(tab, form, Some(sub), prevented);
        }
        match self.link_at(tab, x, y) {
            Some(url) if !prevented => {
                let u = Url::parse(&url).map_err(|e| CoreError::Url(e.to_string()))?;
                self.enqueue_nav_act(tab, DrainAct::Req(Box::new(Request::get(u)), true));
                Ok(InputOutcome::Navigated)
            }
            _ => Ok(InputOutcome::Ignored),
        }
    }

    /// `input`/`textarea` éditable sous `(x, y)` — hit-test +
    /// remontée d'ancêtres (le texte du champ porte le
    /// `NodeId` du texte, pas du champ).
    pub(crate) fn field_at(&self, tab: TabId, x: f32, y: f32) -> Option<NodeId> {
        let entry = self.tabs.get(&tab)?;
        let tree = entry.tree.as_ref()?;
        let dom = entry.dom.as_ref()?;
        let node = hit_test(tree, x, y + entry.scroll_y)?;
        editable_field(dom, node)
    }

    /// `checkbox`/`radio` sous `(x, y)` — hit-test + ancêtres.
    pub(crate) fn toggle_at(&self, tab: TabId, x: f32, y: f32) -> Option<NodeId> {
        let entry = self.tabs.get(&tab)?;
        let (tree, dom) = (entry.tree.as_ref()?, entry.dom.as_ref()?);
        let node = hit_test(tree, x, y + entry.scroll_y)?;
        toggle_control(dom, node)
    }

    /// `<select>` sous `(x, y)` — hit-test + ancêtres.
    pub(crate) fn select_at(&self, tab: TabId, x: f32, y: f32) -> Option<NodeId> {
        let entry = self.tabs.get(&tab)?;
        let (tree, dom) = (entry.tree.as_ref()?, entry.dom.as_ref()?);
        let node = hit_test(tree, x, y + entry.scroll_y)?;
        select_control(dom, node)
    }

    /// Ouvre/ferme le popup `<select>` (étape 54) — options
    /// et boîte figées à l'ouverture ; le nœud ne survit pas
    /// forcément, on ne garde que son chemin.
    pub(crate) fn toggle_select_popup(&mut self, tab: TabId, sel: NodeId) -> Result<(), CoreError> {
        {
            let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
            if entry.popup.is_some() {
                entry.popup = None;
                return self.repaint(tab);
            }
        }
        let entry = self.tabs.get(&tab).ok_or(CoreError::UnknownTab(tab))?;
        let (tree, dom) = (entry.tree.as_ref(), entry.dom.as_ref());
        let (Some(tree), Some(dom)) = (tree, dom) else {
            return Err(CoreError::EmptyDocument);
        };
        let Some(rect) = box_rect(tree, sel) else {
            return Ok(());
        };
        let Some(path) = element_path(dom, sel) else {
            return Ok(());
        };
        let options = descendant_elements(dom, sel, "option")
            .into_iter()
            .map(|o| {
                let label =
                    dom.attribute(o, "label").ok().flatten().unwrap_or_else(|| {
                        dom.text_content(o).unwrap_or_default().trim().to_string()
                    });
                (label, dom.attribute(o, "selected").ok().flatten().is_some())
            })
            .collect();
        let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
        entry.popup = Some(SelectPopup {
            path,
            rect,
            options,
            row_h: rect.height.clamp(14.0, 32.0),
            hovered: None,
        });
        self.repaint(tab)
    }

    /// Clic pendant qu'un popup `<select>` est ouvert — la
    /// ligne choisie applique `selected`, `input`+`change`
    /// partent, le popup se ferme ; un clic hors popup le
    /// ferme simplement. `Some` = clic consommé.
    pub(crate) fn popup_click(
        &mut self,
        tab: TabId,
        x: f32,
        y: f32,
    ) -> Result<Option<InputOutcome>, CoreError> {
        let entry = self.tabs.get(&tab).ok_or(CoreError::UnknownTab(tab))?;
        let Some(pop) = &entry.popup else {
            return Ok(None);
        };
        let row = popup_row(pop, x, y + entry.scroll_y, self.viewport.0, entry.doc_h);
        let path = pop.path.clone();
        // Consommé dans tous les cas : un clic hors popup le
        // referme simplement (comportement natif).
        self.popup_choose(tab, &path, row).map(Some)
    }

    /// `(form, submitter)` sous `(x, y)` — un contrôle submit
    /// dans un `<form>`, `None` sinon.
    pub(crate) fn submit_at(&self, tab: TabId, x: f32, y: f32) -> Option<(NodeId, NodeId)> {
        let entry = self.tabs.get(&tab)?;
        let (tree, dom) = (entry.tree.as_ref()?, entry.dom.as_ref()?);
        let node = hit_test(tree, x, y + entry.scroll_y)?;
        let ctrl = submit_control(dom, node)?;
        let form = enclosing_form(dom, ctrl)?;
        Some((form, ctrl))
    }

    /// `(form, bouton par défaut)` englobant le champ focalisé
    /// — la soumission implicite sur `Enter` ne vaut que pour
    /// les `<input>` mono-ligne (`<textarea>` garde son saut
    /// de ligne). Le bouton par défaut est le premier contrôle
    /// submit du formulaire (HTML §4.10.22.2).
    pub(crate) fn focused_form_default(&self, tab: TabId) -> Option<(NodeId, Option<NodeId>)> {
        let entry = self.tabs.get(&tab)?;
        let dom = entry.dom.as_ref()?;
        let field = entry.focused.as_ref().and_then(|p| dom_path_node(dom, p))?;
        if !dom.tag_name(field).is_ok_and(|t| t == "input") {
            return None;
        }
        let form = enclosing_form(dom, field)?;
        Some((form, default_button(dom, form)))
    }

    /// Touche pendant qu'un popup `<select>` est ouvert :
    /// `ArrowDown`/`ArrowUp` déplacent le surlignage (borné),
    /// `Enter` choisit la ligne surlignée, `Escape` referme —
    /// toutes les autres se ferment sans choix.
    pub(crate) fn popup_key(&mut self, tab: TabId, key: &str) -> Result<InputOutcome, CoreError> {
        let entry = self.tabs.get(&tab).ok_or(CoreError::UnknownTab(tab))?;
        let Some(pop) = &entry.popup else {
            return Ok(InputOutcome::Ignored);
        };
        let rows = pop.options.len().min(MAX_POPUP_ROWS);
        let path = pop.path.clone();
        match key {
            "ArrowDown" | "ArrowUp" => {
                let next = match pop.hovered {
                    Some(h) if key == "ArrowDown" => (h + 1).min(rows.saturating_sub(1)),
                    Some(h) => h.saturating_sub(1),
                    None if key == "ArrowDown" => 0,
                    None => rows.saturating_sub(1),
                };
                let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
                if let Some(pop) = entry.popup.as_mut() {
                    pop.hovered = Some(next);
                }
                self.repaint(tab)?;
                self.emit_invalidated(tab);
                Ok(InputOutcome::Edited)
            }
            "Enter" => {
                let row = pop
                    .hovered
                    .or_else(|| pop.options.iter().position(|&(_, sel)| sel));
                self.popup_choose(tab, &path, row)
            }
            _ => {
                let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
                entry.popup = None;
                self.repaint(tab)?;
                self.emit_invalidated(tab);
                Ok(InputOutcome::Ignored)
            }
        }
    }

    /// Choix `row` dans le popup `<select>` identifié par
    /// `path` — ferme le popup, applique `selected`,
    /// `input`+`change` (queue commune clic/clavier).
    pub(crate) fn popup_choose(
        &mut self,
        tab: TabId,
        path: &[(String, u32)],
        row: Option<usize>,
    ) -> Result<InputOutcome, CoreError> {
        {
            let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
            entry.popup = None;
        }
        let sel = row.and_then(|_| self.resolve_path(tab, Some(path)));
        if let (Some(sel), Some(idx)) = (sel, row) {
            let opts = {
                let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
                let dom = entry.dom.as_mut().ok_or(CoreError::EmptyDocument)?;
                choose_option(dom, sel, idx);
                descendant_elements(dom, sel, "option")
            };
            // Le miroir JS reflète le nouveau `selected` — un
            // handler `change` qui lit `option.selected` voit
            // l'état post-choix (comme `flip_checked`).
            for (i, &o) in opts.iter().enumerate() {
                self.sync_mirror_attr(tab, o, "selected", (i == idx).then_some(""));
            }
            self.relayout(tab)?;
            self.dispatch_path_event(tab, "input", path);
            self.dispatch_path_event(tab, "change", path);
        } else {
            self.repaint(tab)?;
        }
        self.emit_invalidated(tab);
        Ok(InputOutcome::Edited)
    }

    /// Séquence de soumission complète (HTML §4.10.22) —
    /// validation de contraintes, événement `submit`,
    /// action par défaut. Partagée entre le clic sur un
    /// contrôle submit (`click_prevented` = `preventDefault`
    /// du `click`) et la soumission implicite `Enter`
    /// (`submitter` = bouton par défaut du formulaire).
    ///
    /// `novalidate` sur le `<form>` ou `formnovalidate` sur
    /// le `submitter` sautent la validation (HTML).
    pub(crate) fn try_submit(
        &mut self,
        tab: TabId,
        form: NodeId,
        submitter: Option<NodeId>,
        click_prevented: bool,
    ) -> Result<InputOutcome, CoreError> {
        let (form_path, sub_path, novalidate) = {
            let dom = self.tabs.get(&tab).and_then(|e| e.dom.as_ref());
            let nv = dom.is_some_and(|d| {
                d.attribute(form, "novalidate").ok().flatten().is_some()
                    || submitter
                        .is_some_and(|s| d.attribute(s, "formnovalidate").ok().flatten().is_some())
            });
            (
                dom.and_then(|d| element_path(d, form)),
                dom.and_then(|d| submitter.and_then(|s| element_path(d, s))),
                nv,
            )
        };
        // Validation de contraintes (étape 54) : chaque
        // contrôle candidat invalide reçoit `invalid`
        // (non-annulable) puis la soumission est bloquée —
        // `submit` ne part que si tout est valide.
        if !click_prevented && !novalidate {
            let invalid = {
                let dom = self.tabs.get(&tab).and_then(|e| e.dom.as_ref());
                dom.map(|d| invalid_controls(d, form)).unwrap_or_default()
            };
            if !invalid.is_empty() {
                for &ctrl in &invalid {
                    self.dispatch_dom_event(tab, "invalid", ctrl);
                }
                return Ok(InputOutcome::Ignored);
            }
        }
        // `form`/`sub` sont re-résolus après le dispatch — un
        // handler peut re-parser le document (ids morts,
        // chemins survivants).
        if !click_prevented && !self.dispatch_dom_event(tab, "submit", form) {
            self.collect_script_navs(tab);
            if self.nav_pending(tab) {
                return Ok(InputOutcome::Navigated);
            }
            let f = self.resolve_path(tab, form_path.as_deref());
            let s = self.resolve_path(tab, sub_path.as_deref());
            if let Some(f) = f {
                self.submit_form(tab, f, s)?;
                return Ok(InputOutcome::Navigated);
            }
        }
        Ok(InputOutcome::Ignored)
    }

    /// Soumission de formulaire (étape 38) : contrôles
    /// successful → `GET` (query) ou `POST` urlencodé —
    /// posée en intention `pending_nav` (étape 53d : jamais
    /// de fetch sous `&mut self`, le drain de l'appelant
    /// conduit le pipeline complet sur la réponse).
    /// L'historique retient l'URL d'action — un reload/back
    /// refait un `GET` (déviation documentée ADR-0014).
    pub(crate) fn submit_form(
        &mut self,
        tab: TabId,
        form: NodeId,
        submitter: Option<NodeId>,
    ) -> Result<(), CoreError> {
        let req = {
            let entry = self.tabs.get(&tab).ok_or(CoreError::UnknownTab(tab))?;
            let (Some(dom), Some(base)) = (entry.dom.as_ref(), entry.url.as_ref()) else {
                return Err(CoreError::EmptyDocument);
            };
            let pairs = collect_form_data(dom, form, submitter);
            let action = dom
                .attribute(form, "action")
                .ok()
                .flatten()
                .unwrap_or_default();
            let method = dom
                .attribute(form, "method")
                .ok()
                .flatten()
                .unwrap_or_default();
            let enctype = attr_lower(dom, form, "enctype");
            form_request(base, &action, &method, &enctype, &pairs)
                .ok_or_else(|| CoreError::Url("action de formulaire irrésolvable".to_string()))?
        };
        self.enqueue_nav_act(tab, DrainAct::Req(Box::new(req), true));
        Ok(())
    }

    /// Édition du champ focalisé : `Backspace` efface,
    /// `Enter`/`Escape` sortent du champ, un caractère
    /// imprimable s'insère — puis `input` dispatché (valeur
    /// synchronisée au miroir d'abord), restyle + repaint.
    pub(crate) fn handle_key(&mut self, tab: TabId, key: &str) -> Result<InputOutcome, CoreError> {
        let Some(field) = self.focused_node(tab) else {
            tracing::trace!(tab, key, "touche sans champ focalisé");
            return Ok(InputOutcome::Ignored);
        };
        match key {
            "Enter" | "Escape" => {
                self.set_focus(tab, None);
                return Ok(InputOutcome::Ignored);
            }
            _ => {}
        }
        let value = {
            let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
            let dom = entry.dom.as_mut().ok_or(CoreError::EmptyDocument)?;
            edit_field(dom, field, key)?;
            field_value(dom, field)
        };
        // Le handler `input` lit `.value` — le miroir est
        // synchronisé avant le dispatch (édition hors script).
        self.sync_mirror_attr(tab, field, "value", Some(&value));
        self.dispatch_dom_event(tab, "input", field);
        self.relayout(tab)?;
        self.emit_invalidated(tab);
        Ok(InputOutcome::Edited)
    }

    /// Champ focalisé résolu dans `dom` courant — `None` si
    /// le chemin stocké ne mène plus à un élément (le nœud a
    /// été supprimé par un handler ou un re-parse l'a
    /// déplacé).
    pub(crate) fn focused_node(&self, tab: TabId) -> Option<NodeId> {
        self.resolve_path(tab, self.tabs.get(&tab)?.focused.as_deref())
    }

    /// Focus champ (étape 52) : `blur` sur l'ancien nœud puis
    /// `focus` sur le nouveau — ordre navigateur. `None` =
    /// perte de focus seule. Les événements sont dispatchés
    /// par **chemin** : un handler `blur` qui mute le DOM
    /// re-parse le document — la cible `focus` est re-résolue
    /// ensuite sur l'arbre courant.
    pub(crate) fn set_focus(&mut self, tab: TabId, field: Option<NodeId>) {
        let entry = self.tabs.get(&tab);
        let new_path = field.and_then(|f| {
            entry
                .and_then(|e| e.dom.as_ref())
                .and_then(|d| element_path(d, f))
        });
        let old_path = entry.and_then(|e| e.focused.clone());
        if old_path == new_path {
            return;
        }
        if let Some(o) = old_path {
            self.dispatch_path_event(tab, "blur", &o);
        }
        if let Some(entry) = self.tabs.get_mut(&tab) {
            entry.focused.clone_from(&new_path);
        }
        if let Some(p) = new_path {
            self.dispatch_path_event(tab, "focus", &p);
        }
    }

    /// Bascule `checked` du contrôle dans le DOM hôte puis
    /// pousse l'attribut au miroir (étape 52 — l'activation
    /// précède le `click`, le revert suit `preventDefault`).
    pub(crate) fn flip_checked(&mut self, tab: TabId, ctrl: NodeId) -> Result<(), CoreError> {
        let checked = {
            let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
            let dom = entry.dom.as_mut().ok_or(CoreError::EmptyDocument)?;
            toggle_checked(dom, ctrl)?;
            dom.attribute(ctrl, "checked").ok().flatten().is_some()
        };
        self.sync_mirror_attr(tab, ctrl, "checked", checked.then_some(""));
        Ok(())
    }

    /// Nœud DOM sous `(x, y)` viewport — hit-test sur l'arbre
    /// posé, scroll compensé.
    pub(crate) fn node_at(&self, tab: TabId, x: f32, y: f32) -> Option<NodeId> {
        let entry = self.tabs.get(&tab)?;
        let tree = entry.tree.as_ref()?;
        hit_test(tree, x, y + entry.scroll_y)
    }

    /// `href` résolu du lien `<a>` sous `(x, y)` viewport —
    /// hit-test profond sur la boîte posée + remontée des
    /// ancêtres DOM.
    pub(crate) fn link_at(&self, tab: TabId, x: f32, y: f32) -> Option<String> {
        let entry = self.tabs.get(&tab)?;
        let tree = entry.tree.as_ref()?;
        let dom = entry.dom.as_ref()?;
        let node = hit_test(tree, x, y + entry.scroll_y)?;
        let href = ancestor_href(dom, node)?;
        let base = entry.url.as_ref()?;
        Some(Url::resolve(base, &href).ok()?.to_string())
    }

    /// Scroll borné `[0, doc_h - viewport_h]` puis repaint +
    /// `frame_invalidated`.
    pub(crate) fn scroll_by(&mut self, tab: TabId, dy: f32) -> Result<(), CoreError> {
        {
            let entry = self.tabs.get_mut(&tab).ok_or(CoreError::UnknownTab(tab))?;
            let max = (entry.doc_h - self.viewport.1).max(0.0);
            let next = (entry.scroll_y + dy).clamp(0.0, max);
            if (next - entry.scroll_y).abs() < f32::EPSILON {
                return Ok(());
            }
            entry.scroll_y = next;
        }
        self.repaint(tab)?;
        self.emit_invalidated(tab);
        Ok(())
    }
}
