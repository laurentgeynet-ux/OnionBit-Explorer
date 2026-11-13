// This file is part of Explorer.
// Copyright (C) 2026 Laurent Geynet
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Consentement CMP (Étape 171, §B-4 bras CMP — ADR-0031) :
//! politique de consentement de la session, encodeur/décodeur de
//! la **TC string** IAB TCF v2 (segment « core ») et graines de
//! cookies posées avant chaque navigation.
//!
//! # Pourquoi
//!
//! La quasi-totalité des sites européens du corpus de vol pose un
//! mur de consentement dont la résolution pilote le boot : les
//! gates lisent `document.cookie`/`__tcfapi` avant d'insérer
//! contenu et `<video>` — sans état consenti, la page meurt en
//! `evals_sans_mutation`. Plutôt que rejouer chaque SDK (Didomi,
//! `OneTrust`, Sourcepoint…), le moteur porte une **décision
//! explicite** : les cookies attendus sont semés dans le jar
//! avant la requête top-level (état « utilisateur déjà décidé »,
//! côté serveur comme côté script), et le résolveur d'APIs de
//! l'étape suivante répond la même décision.
//!
//! # Honnêteté (bornes assumées)
//!
//! - `EXPLORER_CMP_ID = 0` : le moteur n'est **pas** une CMP
//!   enregistrée auprès de l'IAB — l'ID reste à 0 plutôt que
//!   d'usurper un identifiant alloué. `vendor_list_version = 0`
//!   pareillement (aucune GVL réelle derrière les bitfields).
//! - `RejectAll` n'est pas un mensonge : les champs encodés
//!   disent « rien de consenti » — les sites débloquent leur
//!   contenu fonctionnel et coupent légitimement trackers/pubs,
//!   exactement comme après un vrai « Tout refuser ».
//! - `publisher_cc = "AA"` : le pays de l'éditeur n'est pas
//!   déduit — champ opaque laissé à la convention « inconnu ».

use std::time::{Duration, SystemTime};

/// Décision de consentement portée par la session — injectée
/// via `SessionConfig::consent_policy` (aucune valeur en dur,
/// règle 13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ConsentPolicy {
    /// Aucun état consenti (statu quo ante Étape 171) : zéro
    /// graine, `__tcfapi` reste le descripteur CMP-less — les
    /// murs se comportent comme un utilisateur qui n'a jamais
    /// répondu.
    Honest,
    /// « Tout refuser » (défaut — ADN vie privée du projet) :
    /// les gates se résolvent, le contenu fonctionnel débloque,
    /// trackers et publicités restent légitimement coupés. Les
    /// murs « consent or pay » tiennent — verdict distinct
    /// `consent_wall:hard` dans les rapports de vol, pas une
    /// régression.
    #[default]
    RejectAll,
    /// « Tout accepter » — outil de diagnostic : maximalise la
    /// compatibilité au prix d'un consentement déclaré à tous
    /// purposes/vendors encodés.
    AcceptAll,
}

/// `cmpId` IAB du moteur — 0 = non enregistré (marqueur honnête,
/// aligné sur le descripteur `ping` du stub `__tcfapi`, JS-C213).
pub const EXPLORER_CMP_ID: u16 = 0;

/// `tcfPolicyVersion` encodé — TCF v2.2 courante, même valeur
/// que le descripteur `ping` du stub `__tcfapi`.
pub const TC_POLICY_VERSION: u8 = 4;

/// `MaxVendorId` des bitfields vendor des graines — ordre de
/// grandeur d'une GVL réelle (~1000 vendors) : un site qui
/// re-décode la chaîne trouve une table pleine, toute à la
/// valeur de la politique.
pub const DEFAULT_MAX_VENDOR_ID: usize = 1024;

/// Durée de vie des graines (`Max-Age`) — 180 jours, ordre des
/// cookies CMP réels (OneTrust/Didomi : 6–13 mois). Persistés
/// via le profil, ils gardent l'utilisateur « déjà décidé »
/// entre sessions.
pub const CONSENT_SEED_MAX_AGE_SECS: u64 = 180 * 86_400;

/// `numPubRestriction` borne haute acceptée au décodage — la
/// spec n'en fixe pas, le web est hostile : au-delà, la chaîne
/// est rejetée plutôt que d'allouer sans borne.
pub const MAX_PUB_RESTRICTIONS: usize = 256;

/// Version IAB de la chaîne émise — TCF v2.
const TC_VERSION: u64 = 2;

// ----------------------------------------------------------------
// TC string v2 — segment « core » (IAB TCF spec §Core String)
// ----------------------------------------------------------------

/// Contenu du segment « core » d'une TC string v2 — les champs
/// réellement relus côté site (versions, langue, bitfields
/// purpose/vendor) sont tous portés.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TcConsent {
    /// `created` — décisecondes depuis l'epoch Unix.
    pub created_ds: u64,
    /// `lastUpdated` — décisecondes depuis l'epoch Unix.
    pub last_updated_ds: u64,
    /// `cmpId` (12 bits).
    pub cmp_id: u16,
    /// `cmpVersion` (12 bits).
    pub cmp_version: u16,
    /// `consentScreen` (6 bits).
    pub consent_screen: u8,
    /// `consentLanguage` — deux lettres ASCII majuscules.
    pub consent_language: [u8; 2],
    /// `vendorListVersion` (12 bits).
    pub vendor_list_version: u16,
    /// `tcfPolicyVersion` (6 bits).
    pub tcf_policy_version: u8,
    /// `isServiceSpecific`.
    pub is_service_specific: bool,
    /// `useNonStandardStacks`.
    pub use_non_standard_stacks: bool,
    /// `specialFeatureOptIns` — bit 0 = special feature 1.
    pub special_feature_optins: u16,
    /// `purposeConsents` — bit 0 = purpose 1 (24 bits).
    pub purpose_consents: u32,
    /// `purposeLegitimateInterests` — bit 0 = purpose 1.
    pub purpose_legitimate_interests: u32,
    /// `purposeOneTreatment`.
    pub purpose_one_treatment: bool,
    /// `publisherCC` — deux lettres ASCII majuscules.
    pub publisher_cc: [u8; 2],
    /// `vendorConsents` en bitfield — la longueur du vecteur
    /// est `maxVendorId` (entrée `i` = vendor `i + 1`).
    pub vendor_consents: Vec<bool>,
    /// `vendorLegitimateInterests` — même forme.
    pub vendor_legitimate_interests: Vec<bool>,
    /// `publisherRestrictions` décodées — vide pour nos
    /// graines (émission supportée pour la symétrie du
    /// round-trip).
    pub publisher_restrictions: Vec<PublisherRestriction>,
}

/// Une restriction éditeur (`publisherRestrictions` du segment
/// core) : `purposeId`, `restrictionType`, liste d'entrées —
/// chaque entrée = vendor unique ou intervalle fermé.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublisherRestriction {
    /// `purposeId` (6 bits).
    pub purpose_id: u8,
    /// `restrictionType` (2 bits).
    pub restriction_type: u8,
    /// `(début, fin)` inclusive ; `début == fin` = vendor seul.
    pub ranges: Vec<(u16, u16)>,
}

/// Consentement formé par la politique — `RejectAll` = tout
/// bitfield à faux, `AcceptAll` = tout à vrai. `Honest` produit
/// le même neutre que `RejectAll` (la politique n'émet
/// simplement aucune graine — voir [`seed_headers`]).
#[must_use]
pub fn tc_consent_for_policy(policy: ConsentPolicy, now: SystemTime) -> TcConsent {
    let agreed = matches!(policy, ConsentPolicy::AcceptAll);
    let ds = u64::try_from(
        now.duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_millis()
            .saturating_div(100),
    )
    .unwrap_or(u64::MAX);
    TcConsent {
        created_ds: ds,
        last_updated_ds: ds,
        cmp_id: EXPLORER_CMP_ID,
        cmp_version: 1,
        consent_screen: 0,
        consent_language: *b"EN",
        vendor_list_version: 0,
        tcf_policy_version: TC_POLICY_VERSION,
        is_service_specific: false,
        use_non_standard_stacks: false,
        special_feature_optins: if agreed { 0x0FFF } else { 0 },
        purpose_consents: if agreed { 0x00FF_FFFF } else { 0 },
        purpose_legitimate_interests: if agreed { 0x00FF_FFFF } else { 0 },
        purpose_one_treatment: false,
        publisher_cc: *b"AA",
        vendor_consents: vec![agreed; DEFAULT_MAX_VENDOR_ID],
        vendor_legitimate_interests: vec![agreed; DEFAULT_MAX_VENDOR_ID],
        publisher_restrictions: Vec::new(),
    }
}

/// Écriture de bits MSB-first — accumulateur du segment core.
struct BitWriter(Vec<bool>);

impl BitWriter {
    /// Pousse `n` bits de `v`, poids fort d'abord.
    fn push(&mut self, v: u64, n: usize) {
        for i in (0..n).rev() {
            self.0.push(v >> i & 1 == 1);
        }
    }

    /// Pousse un bit.
    fn bit(&mut self, b: bool) {
        self.0.push(b);
    }
}

const B64URL: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Encode le segment core en base64url sans padding (TC spec :
/// chaque caractère porte 6 bits, complétion par des zéros).
#[must_use]
pub fn encode_tc_string(tc: &TcConsent) -> String {
    let mut w = BitWriter(Vec::new());
    w.push(TC_VERSION, 6);
    w.push(tc.created_ds, 36);
    w.push(tc.last_updated_ds, 36);
    w.push(u64::from(tc.cmp_id & 0x0FFF), 12);
    w.push(u64::from(tc.cmp_version & 0x0FFF), 12);
    w.push(u64::from(tc.consent_screen & 0x3F), 6);
    w.push(u64::from(lang_bits(tc.consent_language)), 12);
    w.push(u64::from(tc.vendor_list_version & 0x0FFF), 12);
    w.push(u64::from(tc.tcf_policy_version & 0x3F), 6);
    w.bit(tc.is_service_specific);
    w.bit(tc.use_non_standard_stacks);
    w.push(u64::from(tc.special_feature_optins & 0x0FFF), 12);
    w.push(u64::from(tc.purpose_consents & 0x00FF_FFFF), 24);
    w.push(u64::from(tc.purpose_legitimate_interests & 0x00FF_FFFF), 24);
    w.bit(tc.purpose_one_treatment);
    w.push(u64::from(lang_bits(tc.publisher_cc)), 12);
    for vendors in [&tc.vendor_consents, &tc.vendor_legitimate_interests] {
        w.push(vendors.len() as u64 & 0xFFFF, 16);
        w.bit(false); // isRangeEncoding = bitfield
        for &b in vendors {
            w.bit(b);
        }
    }
    w.push(tc.publisher_restrictions.len() as u64 & 0x0FFF, 12);
    for r in &tc.publisher_restrictions {
        w.push(u64::from(r.purpose_id & 0x3F), 6);
        w.push(u64::from(r.restriction_type & 0x03), 2);
        w.push(r.ranges.len() as u64 & 0x0FFF, 12);
        for &(start, end) in &r.ranges {
            let is_range = end > start;
            w.bit(is_range);
            w.push(u64::from(start), 16);
            if is_range {
                w.push(u64::from(end), 16);
            }
        }
    }
    let mut out = String::with_capacity(w.0.len() / 6 + 1);
    for chunk in w.0.chunks(6) {
        let mut v = 0u8;
        for &b in chunk {
            v = v << 1 | u8::from(b);
        }
        v <<= 6 - chunk.len();
        if let Some(&c) = B64URL.get(usize::from(v)) {
            out.push(char::from(c));
        }
    }
    out
}

/// `A`..`Z` → 0..=25 ; toute autre valeur retombe sur `A`
/// (tolérant — le champ est opaque).
fn lang_bits(pair: [u8; 2]) -> u16 {
    let f = |c: u8| u16::from(c.wrapping_sub(b'A') % 26);
    f(pair[0]) << 6 | f(pair[1])
}

/// 0..=25 → `A`..`Z`.
fn bits_lang(pair: u64) -> [u8; 2] {
    [
        b'A' + u8::try_from(pair >> 6 & 0x3F).unwrap_or(0),
        b'A' + u8::try_from(pair & 0x3F).unwrap_or(0),
    ]
}

/// Décode le segment core d'une TC string v2 (premier segment
/// d'une chaîne multi-segments). `None` si tronquée ou version
/// inattendue — tolérant aux chaînes étrangères : les sections
/// vendors à `isRangeEncoding` et les restrictions éditeur sont
/// lues, jamais allouées au-delà de [`MAX_PUB_RESTRICTIONS`].
#[must_use]
pub fn decode_tc_string(s: &str) -> Option<TcConsent> {
    let core = s.split('.').next().unwrap_or_default();
    let mut bits = Vec::with_capacity(core.len() * 6);
    for c in core.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => return None,
        };
        for i in (0..6).rev() {
            bits.push(v >> i & 1 == 1);
        }
    }
    let mut r = BitReader(&bits);
    if r.take(6)? != TC_VERSION {
        return None;
    }
    let created_ds = r.take(36)?;
    let last_updated_ds = r.take(36)?;
    let cmp_id = u16::try_from(r.take(12)?).ok()?;
    let cmp_version = u16::try_from(r.take(12)?).ok()?;
    let consent_screen = u8::try_from(r.take(6)?).ok()?;
    let consent_language = bits_lang(r.take(12)?);
    let vendor_list_version = u16::try_from(r.take(12)?).ok()?;
    let tcf_policy_version = u8::try_from(r.take(6)?).ok()?;
    let is_service_specific = r.flag()?;
    let use_non_standard_stacks = r.flag()?;
    let special_feature_optins = u16::try_from(r.take(12)?).ok()?;
    let purpose_consents = u32::try_from(r.take(24)?).ok()?;
    let purpose_legitimate_interests = u32::try_from(r.take(24)?).ok()?;
    let purpose_one_treatment = r.flag()?;
    let publisher_cc = bits_lang(r.take(12)?);
    let vendor_consents = vendor_section(&mut r)?;
    let vendor_legitimate_interests = vendor_section(&mut r)?;
    let n_restrictions = usize::try_from(r.take(12)?).ok()?;
    if n_restrictions > MAX_PUB_RESTRICTIONS {
        return None;
    }
    let mut publisher_restrictions = Vec::with_capacity(n_restrictions);
    for _ in 0..n_restrictions {
        let purpose_id = u8::try_from(r.take(6)?).ok()?;
        let restriction_type = u8::try_from(r.take(2)?).ok()?;
        let n_entries = usize::try_from(r.take(12)?).ok()?;
        let mut ranges = Vec::with_capacity(n_entries.min(1_024));
        for _ in 0..n_entries {
            let is_range = r.flag()?;
            let start = u16::try_from(r.take(16)?).ok()?;
            let end = if is_range {
                u16::try_from(r.take(16)?).ok()?
            } else {
                start
            };
            ranges.push((start, end));
        }
        publisher_restrictions.push(PublisherRestriction {
            purpose_id,
            restriction_type,
            ranges,
        });
    }
    Some(TcConsent {
        created_ds,
        last_updated_ds,
        cmp_id,
        cmp_version,
        consent_screen,
        consent_language,
        vendor_list_version,
        tcf_policy_version,
        is_service_specific,
        use_non_standard_stacks,
        special_feature_optins,
        purpose_consents,
        purpose_legitimate_interests,
        purpose_one_treatment,
        publisher_cc,
        vendor_consents,
        vendor_legitimate_interests,
        publisher_restrictions,
    })
}

/// Lecture de bits MSB-first — miroir de [`BitWriter`].
struct BitReader<'a>(&'a [bool]);

impl BitReader<'_> {
    /// Lit `n` bits, `None` si la chaîne est tronquée.
    fn take(&mut self, n: usize) -> Option<u64> {
        if self.0.len() < n {
            return None;
        }
        let mut v = 0u64;
        for &b in &self.0[..n] {
            v = v << 1 | u64::from(b);
        }
        self.0 = &self.0[n..];
        Some(v)
    }

    /// Lit un bit.
    fn flag(&mut self) -> Option<bool> {
        let b = *self.0.first()?;
        self.0 = &self.0[1..];
        Some(b)
    }
}

/// Section vendor du segment core : `maxVendorId` + soit un
/// bitfield, soit une liste d'intervalles (`isRangeEncoding`).
fn vendor_section(r: &mut BitReader<'_>) -> Option<Vec<bool>> {
    let max = usize::try_from(r.take(16)?).ok()?;
    let mut out = vec![false; max];
    if r.flag()? {
        // Encodage par intervalles : numEntries puis
        // (isRange, start[, end]) par entrée — borne haute
        // structurelle 4 095 (champ 12 bits), l'épuisement
        // des bits rejette de lui-même.
        let n = usize::try_from(r.take(12)?).ok()?;
        for _ in 0..n {
            let is_range = r.flag()?;
            let start = usize::try_from(r.take(16)?).ok()?;
            let end = if is_range {
                usize::try_from(r.take(16)?).ok()?
            } else {
                start
            };
            // IDs 1-based → indices 0-based : vendor `v` =
            // slot `v - 1`. Intervalle fermé borné à
            // `maxVendorId` ; `start > end` → rien.
            let lo = start.saturating_sub(1);
            for slot in out.iter_mut().take(end.min(max)).skip(lo) {
                *slot = true;
            }
        }
    } else {
        for slot in &mut out {
            *slot = r.flag()?;
        }
    }
    Some(out)
}

// ----------------------------------------------------------------
// Graines de cookies de consentement
// ----------------------------------------------------------------

/// En-têtes `Set-Cookie` des graines de consentement pour
/// `policy` — complets (`Path=/`, `SameSite=Lax`, `Max-Age`
/// [`CONSENT_SEED_MAX_AGE_SECS`]) : il suffit de les passer à
/// [`crate::cookies::CookieJar::set_from_header_ctx`] avec l'URL
/// du document — l'hôte y est attaché par le jar.
///
/// Table limitée aux noms **lus par les call-sites du corpus**
/// (règle « bornage terrain ») : `euconsent-v2` (IAB TCF v2),
/// `OptanonConsent`/`OptanonAlertBoxClosed` (`OneTrust` — le
/// second est le « bannière déjà refermée » qui saute le mur),
/// `notice_behavior` (Didomi — `expressed` = choix explicite),
/// `cookieconsent_status` (Cookiebot/Cybot), `usprivacy`
/// (USP/CCPA). Les formats plus exotiques (`didomi_token`
/// signé, `sp_`/`consentUUID` Sourcepoint encodés) ne sont pas
/// forgés — une valeur invalide y serait pire qu'une absence.
///
/// `host` filtre les graines **spécifiques à un domaine** :
/// `SOCS` (le proto sérialisé du bump Google, lu par
/// `ytd-consent-bump*` — `CONSENT` n'est plus relu par
/// `YouTube`) n'est émis que sur [`is_google_consent_host`],
/// jamais sur un hôte tiers où il serait un cookie fantôme.
/// `Honest` → vecteur vide (statu quo).
#[must_use]
pub fn seed_headers(policy: ConsentPolicy, now: SystemTime, host: &str) -> Vec<String> {
    if matches!(policy, ConsentPolicy::Honest) {
        return Vec::new();
    }
    let agreed = matches!(policy, ConsentPolicy::AcceptAll);
    let tc = encode_tc_string(&tc_consent_for_policy(policy, now));
    let iso = iso8601_utc(now);
    let attrs = format!("; Path=/; SameSite=Lax; Max-Age={CONSENT_SEED_MAX_AGE_SECS}");
    let groups = if agreed {
        "C0001:1,C0002:1,C0003:1,C0004:1,C0005:1"
    } else {
        "C0001:1,C0002:0,C0003:0,C0004:0,C0005:0"
    };
    let mut out = vec![
        format!("euconsent-v2={tc}{attrs}"),
        format!("OptanonAlertBoxClosed={iso}{attrs}"),
        format!(
            "OptanonConsent=isGpcEnabled=0&datestamp={iso}&version=202301.1.0&groups={groups}&isIABGlobal=false{attrs}"
        ),
        format!("notice_behavior=expressed,eu{attrs}"),
        format!("cookieconsent_status={}{attrs}", if agreed { "allow" } else { "deny" }),
        format!("usprivacy={}{attrs}", if agreed { "1YNN" } else { "1YYY" }),
    ];
    if is_google_consent_host(host) {
        let socs = if agreed {
            SOCS_ACCEPT_ALL
        } else {
            SOCS_REJECT_ALL
        };
        out.push(format!("SOCS={socs}{attrs}"));
    }
    out
}

/// `SOCS` posé par « Tout refuser » sur le bump Google —
/// proto de décision sérialisé (`CAISNQ` = refus explicite,
/// valeur mesurée par uAssets/uBlock sur le bouton réel).
const SOCS_REJECT_ALL: &str =
    "CAISNQgDEitib3FfaWRlbnRpdHlmcm9udGVuZHVpZXJ2ZXJfMjAyMzA4MjkuMDdfcDEaAmVuIAMaBgiAmc-nBg";

/// `SOCS` posé par « Tout accepter » (`CAES` = accord,
/// forme minimale reconnue par le serveur).
const SOCS_ACCEPT_ALL: &str = "CAESEwgDEgk0ODE3Nzk3MjQaAmVuIAEaBgiA_LyaBg";

/// Hôtes dont la CMP est celle de Google — `SOCS` n'y a de
/// sens que là (`youtube.com`/`youtube-nocookie.com` et
/// `google.<tld>` : le bump `ytd-consent-bump*` et le
/// frontal `consent.google.com`).
fn is_google_consent_host(host: &str) -> bool {
    let h = host.trim_end_matches('.');
    h == "youtube.com"
        || h.ends_with(".youtube.com")
        || h == "youtube-nocookie.com"
        || h.ends_with(".youtube-nocookie.com")
        || h.starts_with("google.")
        || h.contains(".google.")
}

/// `SystemTime` → `YYYY-MM-DDTHH:MM:SSZ` (format ISO des
/// datestamps OneTrust/Didomi).
fn iso8601_utc(t: SystemTime) -> String {
    let secs = t
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs();
    let days = secs / 86_400;
    let tod = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        tod / 3600,
        tod % 3600 / 60,
        tod % 60
    )
}

/// Jours depuis 1970-01-01 → `(année, mois, jour)` civil
/// (algorithme de Hinnant — miroir de `days_from_civil` dans
/// `cookies.rs`).
fn civil_from_days(z: u64) -> (u64, u64, u64) {
    let z = z + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    (
        y + u64::from(mp > 9),
        if mp > 9 { mp - 9 } else { mp + 3 },
        doy - (153 * mp + 2) / 5 + 1,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: SystemTime = SystemTime::UNIX_EPOCH;

    #[test]
    fn tc_string_round_trip_reject_all() {
        let tc = tc_consent_for_policy(ConsentPolicy::RejectAll, NOW);
        let s = encode_tc_string(&tc);
        assert!(!s.is_empty() && !s.contains('='), "base64url sans padding");
        let back = decode_tc_string(&s);
        assert_eq!(back, Some(tc));
    }

    #[test]
    fn tc_string_round_trip_accept_all() {
        let tc = tc_consent_for_policy(ConsentPolicy::AcceptAll, NOW);
        let back = decode_tc_string(&encode_tc_string(&tc));
        assert_eq!(back, Some(tc));
    }

    #[test]
    fn tc_string_champs_lus_site() {
        // Les champs que les re-décodeurs de CMP relisent :
        // version, policyVersion, consentLang, bitfields.
        let tc = decode_tc_string(&encode_tc_string(&tc_consent_for_policy(
            ConsentPolicy::AcceptAll,
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        )))
        .unwrap();
        assert_eq!(tc.cmp_version, 1);
        assert_eq!(tc.tcf_policy_version, TC_POLICY_VERSION);
        assert_eq!(tc.consent_language, *b"EN");
        assert_eq!(
            tc.purpose_consents & 0x3FF,
            0x3FF,
            "purposes 1-10 consentis"
        );
        assert_eq!(tc.vendor_consents.len(), DEFAULT_MAX_VENDOR_ID);
        assert!(tc.vendor_consents.iter().all(|&b| b));
        // Reject : tout faux, strictement.
        let tc = tc_consent_for_policy(ConsentPolicy::RejectAll, NOW);
        assert_eq!(
            tc.purpose_consents | u32::from(tc.special_feature_optins),
            0
        );
        assert!(!tc.vendor_consents.iter().any(|&b| b));
    }

    #[test]
    fn decode_chaines_etrangeres_ranges() {
        // Une chaîne étrangère avec vendors en ranges +
        // restrictions éditeur se lit sans panic ni boucle.
        let mut w = BitWriter(Vec::new());
        w.push(TC_VERSION, 6);
        w.push(0, 36); // created
        w.push(0, 36); // lastUpdated
        w.push(0, 12); // cmpId
        w.push(0, 12); // cmpVersion
        w.push(0, 6); // consentScreen
        w.push(0, 12); // consentLanguage
        w.push(0, 12); // vendorListVersion
        w.push(0, 6); // tcfPolicyVersion
        w.bit(false); // isServiceSpecific
        w.bit(false); // useNonStandardStacks
        w.push(0, 12); // specialFeatureOptIns
        w.push(0, 24); // purposeConsents
        w.push(0, 24); // purposeLegitimateInterests
        w.bit(false); // purposeOneTreatment
        w.push(0, 12); // publisherCC
        w.push(10, 16); // maxVendorId
        w.bit(true); // ranges : [3..5], [7]
        w.push(2, 12);
        w.bit(true);
        w.push(3, 16);
        w.push(5, 16);
        w.bit(false);
        w.push(7, 16);
        w.push(4, 16); // vendorLI : maxVendorId 4, bitfield 0000
        w.bit(false);
        w.bit(false);
        w.bit(false);
        w.bit(false);
        w.bit(false);
        w.push(1, 12); // 1 restriction : purpose 2, type 1, vendor 4
        w.push(2, 6);
        w.push(1, 2);
        w.push(1, 12);
        w.bit(false);
        w.push(4, 16);
        let s = encode_bits(&w.0);
        let tc = decode_tc_string(&s).unwrap();
        assert_eq!(tc.vendor_consents.len(), 10);
        assert!(tc.vendor_consents[2] && tc.vendor_consents[4]);
        assert!(tc.vendor_consents[6]);
        assert_eq!(tc.publisher_restrictions.len(), 1);
    }

    /// Encode un vecteur de bits en base64url (miroir local du
    /// mappeur de fin d'[`encode_tc_string`]).
    fn encode_bits(bits: &[bool]) -> String {
        bits.chunks(6)
            .map(|chunk| {
                let mut v = 0u8;
                for &b in chunk {
                    v = v << 1 | u8::from(b);
                }
                B64URL[usize::from(v << (6 - chunk.len()))] as char
            })
            .collect()
    }

    #[test]
    fn decode_rejette_tronquee_et_version() {
        assert_eq!(decode_tc_string(""), None);
        let mut w = BitWriter(Vec::new());
        w.push(1, 6); // version 1 (TCF v1, hors scope v2)
        w.push(0, 60);
        assert_eq!(decode_tc_string(&encode_bits(&w.0)), None);
        // Tronquée au milieu du segment core.
        let s = encode_tc_string(&tc_consent_for_policy(ConsentPolicy::RejectAll, NOW));
        assert_eq!(decode_tc_string(&s[..s.len() / 2]), None);
    }

    #[test]
    fn seeds_honest_vides() {
        assert_eq!(
            seed_headers(ConsentPolicy::Honest, NOW, "www.youtube.com").len(),
            0
        );
    }

    #[test]
    fn seeds_portent_la_decision() {
        let headers = seed_headers(ConsentPolicy::RejectAll, NOW, "example.org");
        assert_eq!(headers.len(), 6);
        let joined = headers.join("\n");
        assert!(joined.contains("euconsent-v2="));
        assert!(joined.contains("OptanonAlertBoxClosed="));
        assert!(joined.contains("C0002:0"), "reject : groupes OT à 0");
        assert!(joined.contains("cookieconsent_status=deny"));
        assert!(joined.contains("usprivacy=1YYY"));
        assert!(joined.contains("Max-Age="));
        assert!(
            !joined.contains("SOCS="),
            "SOCS reste scopé Google — pas sur example.org"
        );
        let accept = seed_headers(ConsentPolicy::AcceptAll, NOW, "example.org").join("\n");
        assert!(accept.contains("C0002:1"));
        assert!(accept.contains("cookieconsent_status=allow"));
        // Le TC string du seed re-décode à la politique posée.
        let tc_line = headers[0]
            .split(';')
            .next()
            .and_then(|kv| kv.split_once('='))
            .map(|(_, v)| v.to_string())
            .unwrap_or_default();
        let tc = decode_tc_string(&tc_line).unwrap();
        assert_eq!(tc.purpose_consents, 0);
    }

    #[test]
    fn socs_scope_google_uniquement() {
        let yt = seed_headers(ConsentPolicy::RejectAll, NOW, "www.youtube.com").join("\n");
        assert!(yt.contains("SOCS=CAISNQ"), "reject : variante refus");
        let yt_accept = seed_headers(ConsentPolicy::AcceptAll, NOW, "m.youtube.com").join("\n");
        assert!(yt_accept.contains("SOCS=CAES"), "accept : variante accord");
        assert!(yt_accept.contains("Max-Age="));
        for h in [
            "consent.youtube.com",
            "www.youtube-nocookie.com",
            "www.google.fr",
            "google.com",
        ] {
            let s = seed_headers(ConsentPolicy::RejectAll, NOW, h).join("\n");
            assert!(s.contains("SOCS="), "{h} doit recevoir SOCS");
        }
        for h in ["example.org", "myyoutube.com", "notgoogle.com.evil"] {
            let s = seed_headers(ConsentPolicy::RejectAll, NOW, h).join("\n");
            assert!(!s.contains("SOCS="), "{h} ne doit pas recevoir SOCS");
        }
    }

    #[test]
    fn iso8601_epoch() {
        assert_eq!(iso8601_utc(NOW), "1970-01-01T00:00:00Z");
        // 2025-10-31T16:03:44Z — epoch connu.
        assert_eq!(
            iso8601_utc(SystemTime::UNIX_EPOCH + Duration::from_secs(1_761_926_624)),
            "2025-10-31T16:03:44Z"
        );
    }
}
