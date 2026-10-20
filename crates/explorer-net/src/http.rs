// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Types HTTP/1.1 + trait [`HttpTransport`] injectable (ADR-0002).
//!
//! Le trait est la couture : `explorer-core` ne sait pas comment on
//! ouvre une socket — il reçoit un `Box<dyn HttpTransport>` (le vrai
//! [`crate::h1::H1Client`] en prod, un fake in-process en test).

use async_trait::async_trait;

use crate::error::NetResult;
use crate::url::Url;

/// Méthode HTTP (sous-ensemble usuel — `Other` pour l'exotique).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    /// `GET`.
    Get,
    /// `HEAD`.
    Head,
    /// `POST`.
    Post,
    /// `PUT`.
    Put,
    /// `DELETE`.
    Delete,
    /// `OPTIONS`.
    Options,
}

impl Method {
    /// Verbe tel que sérialisé sur le fil.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Head => "HEAD",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Delete => "DELETE",
            Self::Options => "OPTIONS",
        }
    }
}

/// Une requête HTTP.
#[derive(Debug, Clone)]
pub struct Request {
    /// Verbe.
    pub method: Method,
    /// URL absolue (déjà normalisée par [`Url`]).
    pub url: Url,
    /// En-têtes additionnels (ordre conservé, doublons permis).
    pub headers: Vec<(String, String)>,
    /// Corps (POST/PUT) — `None` = aucun.
    pub body: Option<Vec<u8>>,
}

impl Request {
    /// `GET url` sans en-têtes ni corps.
    #[must_use]
    pub fn get(url: Url) -> Self {
        Self {
            method: Method::Get,
            url,
            headers: Vec::new(),
            body: None,
        }
    }
}

/// Une réponse HTTP/1.1.
///
/// `body` est le corps **déframé** (chunked résolu) mais **pas**
/// décompressé — `Content-Encoding` est l'affaire de l'étape 6.
#[derive(Debug, Clone)]
pub struct Response {
    /// Code de statut (`200`, `404`…).
    pub status: u16,
    /// Raison (`OK`, `Not Found`…).
    pub reason: String,
    /// En-têtes — paires `(nom, valeur)`, **ordre et doublons
    /// préservés** (`Set-Cookie` arrive en plusieurs lignes).
    pub headers: Vec<(String, String)>,
    /// Corps déframé (Content-Length lu / chunked assemblé / EOF).
    pub body: Vec<u8>,
    /// `true` si la connexion a été réutilisée depuis le pool
    /// keep-alive — exposé pour les tests et les métriques.
    pub reused_connection: bool,
    /// URL finale de la ressource quand une chaîne de
    /// redirections a été suivie (`301`/`302`/`303`/`307`/
    /// `308` — `RedirectTransport`), fragment `Location`
    /// inclus ; `None` = URL de la requête (étape 67).
    pub resolved_url: Option<crate::url::Url>,
}

impl Response {
    /// Première valeur d'un en-tête (nom case-insensitive).
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// Toutes les valeurs d'un en-tête.
    pub fn headers_all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.headers
            .iter()
            .filter(move |(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    /// `Content-Type` brut (paramètres compris).
    #[must_use]
    pub fn content_type(&self) -> Option<&str> {
        self.header("content-type")
    }

    /// Charset déclaré dans `Content-Type` (`; charset=x`), en
    /// minuscules — `None` si absent (le sniffing WHATWG est
    /// l'affaire de `explorer-html`, pas du transport).
    #[must_use]
    pub fn charset(&self) -> Option<String> {
        let ct = self.content_type()?;
        ct.split(';').skip(1).find_map(|p| {
            let (k, v) = p.trim().split_once('=')?;
            k.trim()
                .eq_ignore_ascii_case("charset")
                .then(|| v.trim().trim_matches('"').to_ascii_lowercase())
        })
    }

    /// `X-Content-Type-Options: nosniff` — le client doit honorer
    /// le `Content-Type` déclaré (étape 6).
    #[must_use]
    pub fn nosniff(&self) -> bool {
        self.header("x-content-type-options")
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("nosniff"))
    }

    /// Corps transcodé en texte selon `charset` (labels WHATWG via
    /// `encoding_rs`, remplacement U+FFFD sur erreur) ; défaut
    /// `windows-1252` (défaut de la spec HTML).
    #[must_use]
    pub fn text(&self) -> String {
        let label = self.charset().unwrap_or_else(|| "windows-1252".to_string());
        let enc =
            encoding_rs::Encoding::for_label(label.as_bytes()).unwrap_or(encoding_rs::WINDOWS_1252);
        let (s, _, _) = enc.decode(&self.body);
        s.into_owned()
    }
}

/// Réponse à corps **streamé** (étape 53) : statut et en-têtes
/// disponibles dès leur réception, le corps arrive déframé par
/// morceaux sur `body` — la session parse et peint pendant le
/// téléchargement (ADR-0012). Le canal se ferme en fin de corps ;
/// une erreur de lecture est livrée comme dernier item `Err`.
pub struct StreamedResponse {
    /// Code de statut (`200`, `404`…).
    pub status: u16,
    /// Raison (`OK`, `Not Found`…).
    pub reason: String,
    /// En-têtes — paires `(nom, valeur)`, ordre et doublons
    /// préservés.
    pub headers: Vec<(String, String)>,
    /// `true` si la connexion a été réutilisée depuis le pool
    /// keep-alive.
    pub reused_connection: bool,
    /// URL finale après redirections éventuelles — même
    /// sémantique que [`Response::resolved_url`] (étape 67).
    pub resolved_url: Option<crate::url::Url>,
    /// Morceaux de corps déframé — se ferme proprement à EOF.
    pub body: tokio::sync::mpsc::Receiver<NetResult<Vec<u8>>>,
}

impl StreamedResponse {
    /// Première valeur d'un en-tête (nom case-insensitive).
    #[must_use]
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// Port réseau : exécute une requête et rend une réponse déframée.
///
/// Injectable — les tests passent un fake in-process, jamais de
/// socket réel en CI (ADR-0002).
#[async_trait]
pub trait HttpTransport: Send + Sync {
    /// Exécute `req`, renvoie la réponse déframée ou une erreur
    /// typée.
    async fn fetch(&self, req: &Request) -> NetResult<Response>;

    /// Variante streamée de [`HttpTransport::fetch`] (étape 53) :
    /// métadonnées dès les en-têtes, corps par morceaux sur le
    /// canal — la navigation peint pendant le téléchargement.
    /// Implémentation par défaut : `fetch` bufferisé, le corps
    /// arrive en un seul morceau (fakes, transports simples).
    async fn fetch_stream(&self, req: &Request) -> NetResult<StreamedResponse> {
        let resp = self.fetch(req).await?;
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        // Capacité 1 — `try_send` ne bloque pas ; le canal se
        // ferme en quittant la portée (fin de corps signalée).
        let _ = tx.try_send(Ok(resp.body));
        Ok(StreamedResponse {
            status: resp.status,
            reason: resp.reason,
            headers: resp.headers,
            reused_connection: resp.reused_connection,
            resolved_url: resp.resolved_url,
            body: rx,
        })
    }
}
