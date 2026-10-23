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

/// Destination d'une requête (`Sec-Fetch-Dest`, étape 125) —
/// posée par l'émetteur à la création ; `Empty` est le défaut
/// sûr (`fetch()`/`XHR`, outils — la spec leur assigne
/// `empty`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FetchDest {
    /// `document` — navigation de haut niveau.
    Document,
    /// `script` — `<script src>` et modules.
    Script,
    /// `style` — `<link rel=stylesheet>` et `@import`.
    Style,
    /// `image` — `<img>`, fonds CSS, masques.
    Image,
    /// `font` — `@font-face` `url()`.
    Font,
    /// `iframe` — navigation d'un contexte imbriqué
    /// (`<iframe src>`, étape 128).
    Iframe,
    /// `empty` — `fetch()`/`XHR` et requêtes non typées.
    #[default]
    Empty,
}

/// Mode d'une sous-ressource (`Sec-Fetch-Mode`, étape 125).
/// `navigate` y figure (étape 128) : une navigation de
/// contexte **imbriqué** porte l'initiateur `Subresource`
/// (la page parente — CHIPS/`SameSite`) mais un mode
/// `navigate` à l'émission ; les navigations top-level
/// continuent de le dériver de l'initiateur — un appelant
/// ne pose ce mode à la main que pour un sous-contexte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FetchMode {
    /// `cors` — `fetch()`/`XMLHttpRequest` (spec Fetch).
    Cors,
    /// `no-cors` — sous-ressources classiques
    /// (`<img>`/`<script>`/`<link>` — défaut conservateur).
    #[default]
    NoCors,
    /// `same-origin` — garde-fou spec (`Request.mode` interne
    /// des APIs qui exigent la même origine).
    SameOrigin,
    /// `navigate` — navigation d'un sous-contexte
    /// (`<iframe>`, étape 128) ; les navigations top-level
    /// n'en ont pas besoin (dérivé de l'initiateur).
    Navigate,
}

/// Mode `credentials` d'une requête scriptée (étape 126,
/// spec Fetch `Request.credentials`) — gouverne l'envoi du
/// `Cookie` et la lisibilité de la réponse cross-origin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FetchCredentials {
    /// `omit` — jamais de `Cookie` sortant, `Set-Cookie`
    /// entrant ignoré ; la réponse reste lisible sur `ACAO`.
    Omit,
    /// `same-origin` — défaut `fetch()` : credentials pour
    /// la même origine seulement (comportement historique).
    #[default]
    SameOrigin,
    /// `include` — credentials y compris cross-origin ;
    /// la réponse n'est lisible que si `ACAO` exact +
    /// `Access-Control-Allow-Credentials: true` (jamais `*`).
    Include,
}

/// Politique `Referer` d'une requête (étape 132, spec
/// *Referrer-Policy*) — le document la fixe par en-tête
/// `Referrer-Policy:` ou `<meta name=referrer>`, un élément
/// la redéfinit par `referrerpolicy=` ; la valeur `None`
/// champ `Request::referrer_policy` = politique par défaut
/// de la plateforme (`StrictOriginWhenCrossOrigin`,
/// comportement antérieur à l'étape 132).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReferrerPolicy {
    /// `no-referrer` — jamais d'en-tête `Referer`.
    NoReferrer,
    /// `no-referrer-when-downgrade` — URL complète sauf en
    /// rétrogradation `https → http` (ancien défaut du web).
    NoReferrerWhenDowngrade,
    /// `same-origin` — URL complète en same-origin, rien
    /// en cross-origin (rétrogradation incluse).
    SameOrigin,
    /// `origin` — l'origine seule, quel que soit le schéma
    /// de la cible (rétrogradation incluse).
    Origin,
    /// `strict-origin` — l'origine seule, sauf en
    /// rétrogradation `https → http` : rien.
    StrictOrigin,
    /// `origin-when-cross-origin` — URL complète en
    /// same-origin, origine en cross-origin (rétrogradation
    /// incluse).
    OriginWhenCrossOrigin,
    /// `strict-origin-when-cross-origin` — URL complète en
    /// same-origin, origine en cross-origin, rien en
    /// rétrogradation. **Défaut** = comportement d'avant
    /// l'étape 132.
    #[default]
    StrictOriginWhenCrossOrigin,
    /// `unsafe-url` — URL complète, y compris en
    /// rétrogradation.
    UnsafeUrl,
}

impl ReferrerPolicy {
    /// Parse un jeton de politique (spec : liste de valeurs
    /// séparées par des virgules — la **dernière valide**
    /// gagne, une liste sans valeur reconnue → `None`).
    /// `""`/`no-referrer-when-downgrade` non reconnu → ignoré
    /// (le champ garde sa valeur précédente).
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        let mut out = None;
        for tok in value.split(',') {
            out = match tok.trim().to_ascii_lowercase().as_str() {
                "no-referrer" | "never" => Some(Self::NoReferrer),
                "origin" => Some(Self::Origin),
                "origin-when-cross-origin" => Some(Self::OriginWhenCrossOrigin),
                "same-origin" => Some(Self::SameOrigin),
                "strict-origin" => Some(Self::StrictOrigin),
                "strict-origin-when-cross-origin" => Some(Self::StrictOriginWhenCrossOrigin),
                "unsafe-url" | "always" => Some(Self::UnsafeUrl),
                _ => out,
            };
        }
        out
    }
}

/// Contexte CORS d'une requête `mode:cors` cross-origin
/// (étape 126) — posé par la session à l'émission ; le
/// transport de politique l'utilise pour le preflight
/// `OPTIONS` et la vérification de la réponse. `None` =
/// requête non soumise au protocole (navigations, sous-
/// ressources `no-cors`, fetch same-origin).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorsRequest {
    /// Origine canonique de la page émettrice — comparée à
    /// `Access-Control-Allow-Origin` et posée en `Origin:`
    /// du preflight (`scheme://host[:port]`, `"null"` si
    /// opaque — sérialisée par la session).
    pub origin: String,
    /// `credentials:` de l'appel script.
    pub credentials: FetchCredentials,
    /// En-têtes auteur **non** CORS-safelistés — déclencheurs
    /// de preflight et contenu de `Access-Control-Request-
    /// Headers`. La plateforme les connaît à l'émission
    /// (après filtrage des interdits) : le transport ne les
    /// redevine pas dans les en-têtes fusionnés.
    pub unsafe_headers: Vec<String>,
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
    /// Initiateur pour la politique `SameSite` (« site pour
    /// cookies », RFC 6265bis) — `None` = pas de contexte,
    /// envoi permissif (compat — les fakes et outils n'ont
    /// pas de page initiatrice). Posé par la session
    /// (étape 66).
    pub initiator: Option<Initiator>,
    /// Destination `Sec-Fetch-Dest` (étape 125) — posée par
    /// l'émetteur ; `Empty` = non typée.
    pub fetch_dest: FetchDest,
    /// Mode `Sec-Fetch-Mode` d'une sous-ressource (étape 125)
    /// — ignoré pour les navigations (`navigate` est dérivé
    /// de l'initiateur). `fetch()`/`XHR` posent `Cors`.
    pub fetch_mode: FetchMode,
    /// Activation utilisateur (étape 125) — `true` = émission
    /// issue d'un geste (clic, submit, saisie omnibox,
    /// reload) ; `Sec-Fetch-User: ?1` n'est émis que sur les
    /// navigations ainsi marquées. Posée aux points de
    /// dispatch, jamais devinée côté transport.
    pub fetch_user: bool,
    /// Contexte CORS (étape 126) — `Some` sur une requête
    /// `mode:cors` cross-origin : le transport de politique
    /// y lit `origin`/`credentials`/`unsafe_headers` pour le
    /// preflight et la lisibilité de la réponse.
    pub cors: Option<CorsRequest>,
    /// Suppression du `Referer` (étape 131) — `true` quand
    /// la source l'exige (`rel=noreferrer`, politique
    /// `no-referrer`) : `RefererTransport` n'injecte pas
    /// l'en-tête même si un initiateur est suivi.
    /// `Origin` n'est **pas** concerné (il n'est pas une
    /// information de référent).
    pub no_referer: bool,
    /// Politique `Referer` de la requête (étape 132) —
    /// `None` = défaut plateforme
    /// (`StrictOriginWhenCrossOrigin`). Raffinée par le
    /// document (`Referrer-Policy:` / `<meta name=referrer>`)
    /// ou l'élément (`referrerpolicy=`) à l'émission ;
    /// `no_referer` reste prioritaire.
    pub referrer_policy: Option<ReferrerPolicy>,
}

/// Qui a initié la requête — « site pour cookies » de la
/// politique `SameSite` (étape 66).
#[derive(Debug, Clone)]
pub enum Initiator {
    /// Navigation de haut niveau sans page initiatrice
    /// (saisie utilisateur, favori, reload, back/forward) —
    /// le site pour cookies est la cible elle-même :
    /// `Strict`/`Lax`/`None` passent tous.
    TopLevelNav,
    /// Navigation de haut niveau initiée par une page
    /// (lien, formulaire, `location.href`) — `Strict` exige
    /// same-site ; `Lax` passe en plus sur les méthodes
    /// sûres (`GET`/`HEAD`).
    NavFrom(Url),
    /// Sous-ressource ou `fetch`/`XHR` d'une page —
    /// `Strict`/`Lax` n'envoient que same-site ; `None`
    /// reste libre (`Secure` exigé à la pose).
    Subresource(Url),
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
            initiator: None,
            fetch_dest: FetchDest::Empty,
            fetch_mode: FetchMode::NoCors,
            fetch_user: false,
            cors: None,
            no_referer: false,
            referrer_policy: None,
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
