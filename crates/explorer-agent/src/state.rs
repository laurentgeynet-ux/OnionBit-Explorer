// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! État partagé du serveur agent et état par connexion :
//! `AgentShared` (session moteur, clé, révisions, ring console,
//! recorder unique) et `AgentConn` (profil accordé, capacités,
//! souscriptions, table de handles).

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use explorer_core::{CoreEvent, Session, TabId};

use crate::proto::{Capability, ElementRef, EventKind, Profile, TabScope};

/// Capacité par onglet du ring console — même borne que l'API
/// (512 : assez pour un boot de bundle).
const CONSOLE_CAP: usize = 512;

/// Capacité de la table de handles par connexion — au-delà, les
/// plus anciennes refs expirent (`StaleRef` à la résolution,
/// jamais de croissance non bornée sur contenu hostile).
const HANDLES_CAP: usize = 4096;

/// État partagé — une instance par routeur (une par daemon).
pub struct AgentShared {
    /// Session moteur — même verrou que l'API UI (coexistence
    /// ADR-0035, single-writer).
    pub session: Arc<tokio::sync::Mutex<Session>>,
    /// Clé optionnelle — même règle que `?key=`/`x-api-key`.
    pub key: Option<String>,
    /// Révision de document commis par onglet — base
    /// `snapshot{since}`. **Writer unique** : le recorder (un
    /// incrément par `Navigated` de son flux ordonné).
    revs: std::sync::Mutex<HashMap<TabId, u64>>,
    /// Ring des erreurs pipeline par onglet (recorder unique).
    console: std::sync::Mutex<HashMap<TabId, VecDeque<String>>>,
    /// Profil réellement servi par ce daemon (`--profile`/env/
    /// défaut), injecté par l'hôte — annoncé comme clé `profile`
    /// du bloc `dirs` de `session.open` (étape 265) ; `None`
    /// = non injecté (adaptateurs, tests).
    pub served_profile: Option<std::path::PathBuf>,
}

impl AgentShared {
    /// Construit l'état sans profil injecté et arme le recorder :
    /// `Navigating` purge le ring de l'onglet (les erreurs de
    /// boot appartiennent à la nouvelle page), `PipelineError`
    /// l'alimente (cap), `Navigated` incrémente la révision.
    pub fn new(session: Arc<tokio::sync::Mutex<Session>>, key: Option<String>) -> Arc<Self> {
        Self::with_profile(session, key, None)
    }

    /// [`Self::new`] + profil servi injecté — voir
    /// [`AgentShared::served_profile`].
    pub fn with_profile(
        session: Arc<tokio::sync::Mutex<Session>>,
        key: Option<String>,
        served_profile: Option<std::path::PathBuf>,
    ) -> Arc<Self> {
        let st = Arc::new(Self {
            session,
            key,
            served_profile,
            revs: std::sync::Mutex::new(HashMap::new()),
            console: std::sync::Mutex::new(HashMap::new()),
        });
        spawn_recorder(&st);
        st
    }

    /// Révision courante de l'onglet (0 = jamais commit).
    #[must_use]
    pub fn rev(&self, tab: TabId) -> u64 {
        self.revs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&tab)
            .copied()
            .unwrap_or(0)
    }

    /// Entrées console de l'onglet, plus récentes dernières.
    #[must_use]
    pub fn console_entries(&self, tab: TabId, limit: usize) -> Vec<String> {
        let ring = self
            .console
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        ring.get(&tab)
            .map(|q| q.iter().rev().take(limit).rev().cloned().collect())
            .unwrap_or_default()
    }
}

/// Le recorder unique : souscrit au broadcast session et tient
/// `revs` + `console`. Même motif que `spawn_console_recorder`
/// de l'API (`lock().await` pour `subscribe()`, jamais
/// `try_lock` — §B-48).
fn spawn_recorder(st: &Arc<AgentShared>) -> tokio::task::JoinHandle<()> {
    let session = Arc::clone(&st.session);
    let st = Arc::clone(st);
    tokio::spawn(async move {
        let mut rx = session.lock().await.subscribe();
        loop {
            match rx.recv().await {
                Ok(ev) => match ev {
                    CoreEvent::Navigating { tab, .. } => {
                        st.console
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .entry(tab)
                            .or_default()
                            .clear();
                    }
                    CoreEvent::PipelineError {
                        tab: Some(tab),
                        message,
                    } => {
                        let mut g = st
                            .console
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        let q = g.entry(tab).or_default();
                        if q.len() >= CONSOLE_CAP {
                            q.pop_front();
                        }
                        q.push_back(message);
                    }
                    CoreEvent::Navigated { tab, .. } => {
                        *st.revs
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .entry(tab)
                            .or_insert(0) += 1;
                    }
                    _ => {}
                },
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!(dropped = n, "eap recorder — événements perdus sous rafale");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    })
}

// ------------------------------------------------------------------
// Connexion
// ------------------------------------------------------------------

/// Filtre de souscription — lu par le forwarder à chaque
/// événement, écrit par `events.subscribe`/`unsubscribe`.
#[derive(Debug, Default)]
pub struct SubFilter {
    /// Onglet observé — `None` = toute la session.
    pub tab: Option<TabId>,
    /// Familles souscrites.
    pub kinds: HashSet<EventKind>,
}

/// Handle alloué — le `rev` fige la génération du document : une
/// ref ne survit jamais à un commit (re-parse → `NodeId`
/// réattribués, règle ADR-0008/GeomBox).
#[derive(Debug, Clone, Copy)]
pub struct Handle {
    /// Onglet propriétaire.
    pub tab: TabId,
    /// `NodeId` arène de la boîte.
    pub node: u32,
    /// Révision au moment de l'allocation.
    pub rev: u64,
}

/// Table de handles de la connexion — FIFO bornée
/// (borne `HANDLES_CAP`), purge totale à la fermeture d'onglet.
/// **Dédoublonnée** : réémettre `(tab, node, rev)` rend la ref
/// déjà allouée — un élément garde son identité entre snapshots
/// d'une même révision (le diff `changed` cite la ref stable).
#[derive(Debug, Default)]
pub struct HandleTable {
    map: HashMap<ElementRef, Handle>,
    /// Index inverse — dédoublonnage par identité DOM.
    by_node: HashMap<(TabId, u32, u64), ElementRef>,
    order: VecDeque<ElementRef>,
    next: u64,
}

impl HandleTable {
    /// Alloue une ref — réémission dédupliquée, évince la plus
    /// ancienne au cap.
    pub fn alloc(&mut self, tab: TabId, node: u32, rev: u64) -> ElementRef {
        let key = (tab, node, rev);
        if let Some(r) = self.by_node.get(&key) {
            // Ne réapplique que si la ref est toujours vivante
            // (évincée entre-temps → fraîche).
            if self.map.contains_key(r) {
                return r.clone();
            }
        }
        self.next += 1;
        let r = format!("el:{}", self.next);
        if self.map.len() >= HANDLES_CAP {
            if let Some(old) = self.order.pop_front() {
                if let Some(h) = self.map.remove(&old) {
                    self.by_node.remove(&(h.tab, h.node, h.rev));
                }
            }
        }
        self.order.push_back(r.clone());
        self.map.insert(r.clone(), Handle { tab, node, rev });
        self.by_node.insert(key, r.clone());
        r
    }

    /// Résout une ref — `StaleRef` si inconnue, autre onglet ou
    /// document re-commit (`rev` divergeant).
    ///
    /// # Errors
    ///
    /// [`crate::proto::ErrorCode::StaleRef`] — jamais d'autre code :
    /// une ref qui échoue à la résolution est expirée, point.
    pub fn resolve(
        &self,
        elem_ref: &str,
        tab: TabId,
        rev: u64,
    ) -> Result<Handle, crate::proto::ErrorCode> {
        match self.map.get(elem_ref) {
            Some(h) if h.tab == tab && h.rev == rev => Ok(*h),
            _ => Err(crate::proto::ErrorCode::StaleRef),
        }
    }

    /// Purge les refs d'un onglet fermé.
    pub fn purge_tab(&mut self, tab: TabId) {
        self.map.retain(|_, h| h.tab != tab);
        self.by_node.retain(|(t, _, _), _| *t != tab);
        self.order.retain(|r| self.map.contains_key(r));
    }
}

/// Émissions snapshot retenues par onglet — historique
/// borné (étape 254). Une `rev` n'identifie pas
/// l'émission : deux `snapshot.get` à `rev` égale peuvent
/// différer par leurs params (`interactive_only`,
/// `max_nodes`). `snap_seq` (monotone par connexion) la
/// nomme sans ambiguïté — `since_seq` vise l'émission
/// exacte, `since` conserve sa sémantique « dernière
/// émission à cette rev ».
pub const SNAP_HISTORY: usize = 8;

/// Émission snapshot mémorisée — base d'un diff `{since}`
/// ou `{since_seq}` (E3/étape 254).
pub struct StoredSnap {
    /// Séquence d'émission — monotone par connexion.
    pub snap_seq: u64,
    /// `rev` servie — `since == rev` déclenche le diff sur
    /// la dernière émission retenue à cette rev.
    pub rev: u64,
    /// Entrées émises (index+génération+ref+signature).
    pub entries: Vec<crate::snapshot::SnapEntry>,
}

/// État d'une connexion agent — né à l'upgrade, meurt au close.
#[derive(Default)]
pub struct AgentConn {
    /// `session.open` reçu — obligatoire avant tout verbe métier.
    pub opened: bool,
    /// Capacités livrées (demandées ∩ implémentées).
    pub caps: HashSet<Capability>,
    /// Profil de droits accordé.
    pub profile: Profile,
    /// Filtre de souscription partagé avec le forwarder.
    pub subs: Arc<std::sync::RwLock<SubFilter>>,
    /// Table de handles.
    pub handles: HandleTable,
    /// Émissions snapshot retenues par onglet — ring borné
    /// [`SNAP_HISTORY`], bases des diffs `{since}`/`{since_seq}`
    /// (E3/étape 254), purgé avec les handles.
    pub snaps: HashMap<TabId, VecDeque<StoredSnap>>,
    /// Compteur d'émissions snapshot — clé exacte d'un diff
    /// `{since_seq}` (étape 254).
    pub snap_seq: u64,
    /// Fin demandée (`session.close`) — la boucle sort après la
    /// réponse.
    pub closing: bool,
}

impl AgentConn {
    /// `tab` est-il dans la portée du profil ?
    #[must_use]
    pub fn tab_allowed(&self, tab: TabId) -> bool {
        match &self.profile.tabs {
            TabScope::All(_) => true,
            TabScope::Only(ids) => ids.contains(&tab),
        }
    }

    /// `true` si la session profil autorise de nouveaux onglets
    /// (portée `all` — un onglet neuf ne peut pas appartenir à
    /// une liste `Only` préexistante).
    #[must_use]
    pub fn may_open_tab(&self) -> bool {
        matches!(self.profile.tabs, TabScope::All(_))
    }
}
