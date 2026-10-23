// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Transport décorateur : `Accept-Language` piloté à chaud.
//! La valeur vit dans une cellule partagée — l'hôte (session,
//! API `/accept-language`, réglages app) la fait évoluer sans
//! reconstruire la pile de transports (étape 118).

use std::sync::{Arc, RwLock};

use crate::error::NetResult;
use crate::http::{HttpTransport, Request, Response, StreamedResponse};
use async_trait::async_trait;

/// Cellule partagée `Accept-Language` — `None` = en-tête non
/// envoyé. `RwLock` std : la lecture ne traverse jamais un
/// `.await` (clonée puis relâchée avant le fetch).
pub type AcceptLanguage = Arc<RwLock<Option<String>>>;

/// Crée la cellule — `initial` vient de la config de session.
#[must_use]
pub fn accept_language_cell(initial: Option<String>) -> AcceptLanguage {
    Arc::new(RwLock::new(initial))
}

/// Injecte `Accept-Language` dans la requête si la cellule est
/// renseignée et que l'appelant ne l'a pas déjà posée — un
/// `fetch()` avec un en-tête explicite reste prioritaire.
fn inject(req: &Request, lang: &AcceptLanguage) -> Request {
    let v = lang
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let Some(v) = v.filter(|v| !v.is_empty()) else {
        return req.clone();
    };
    if req
        .headers
        .iter()
        .any(|(n, _)| n.eq_ignore_ascii_case("accept-language"))
    {
        return req.clone();
    }
    let mut req = req.clone();
    req.headers.push(("Accept-Language".to_string(), v));
    req
}

/// Transport décorateur — injecte la cellule dans `fetch` et
/// `fetch_stream` : navigations, sous-ressources et requêtes
/// script-initiées annoncent la même langue.
pub struct LangTransport {
    inner: Arc<dyn HttpTransport>,
    lang: AcceptLanguage,
}

impl LangTransport {
    /// Enveloppe `inner` ; `lang` est relu à chaque requête.
    #[must_use]
    pub fn wrap(inner: Arc<dyn HttpTransport>, lang: AcceptLanguage) -> Self {
        Self { inner, lang }
    }
}

#[async_trait]
impl HttpTransport for LangTransport {
    async fn fetch(&self, req: &Request) -> NetResult<Response> {
        self.inner.fetch(&inject(req, &self.lang)).await
    }

    async fn fetch_stream(&self, req: &Request) -> NetResult<StreamedResponse> {
        self.inner.fetch_stream(&inject(req, &self.lang)).await
    }
}
