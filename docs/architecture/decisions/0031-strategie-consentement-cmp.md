# ADR-0031 — Stratégie consentement CMP : politique de session, pas émulation SDK

Date : 2026-11-01 · Statut : **acceptée** — Étape 171 +
JS-C259 (§B-4 bras CMP).

## Contexte

Quasi tous les sites européens du corpus de vol posent un mur
de consentement (privacy-center, OneTrust/`OtAutoBlock`,
Sourcepoint/`__tcfapi`, Didomi, appconsent, Quantcast, Osano)
dont la résolution pilote le boot : les gates lisent
`document.cookie`/`__tcfapi` **avant** d'insérer contenu et
`<video>`. Sans état consenti, les pages meurent en
`evals_sans_mutation` — la chaîne média (MSE vérifiée
bout-en-bout, ADR-0029) reste du code mort.

Trois mécanismes observés au terrain :

1. les SDKs CMP crashent dans notre moteur (§B-14 :
   `id of Undefined` privacy-center, `cloneNode of Null`
   OneTrust…),
2. certains figent l'éval (§B-48 : `OtAutoBlock.js` > 90 s
   fifa),
3. le consentement ne se résout jamais — le stub `__tcfapi`
   honnête (JS-C213) répond « pas de CMP » et les listeners
   attendent éternellement un `tcData` — **résolu en
   JS-C259** sous politique non-`Honest` : résolveur formé
   IAB v2 piloté par la session.

Plus profond : OneTrust/Cookiebot/Didomi servent les vrais
scripts en `type="text/plain"` (+ marqueurs
`optanon-category-*`, `data-cookieconsent`, `data-purpose`) et
ne les réarment qu'au consentement — `collect_scripts` ne les
voit jamais.

## Décision

Le moteur porte une **décision de consentement explicite** par
session — `SessionConfig::consent_policy` — plutôt que de
rejouer chaque SDK :

- **`Honest`** : statu quo. Aucune graine, `__tcfapi` reste
  le descripteur CMP-less. Garde-fou comparatif et seul mode
  pour les mesures compliance-sensibles.
- **`RejectAll`** (défaut) : « Tout refuser ». Grainé avant la
  requête top-level : `euconsent-v2` (TC string IAB v2 réelle
  — encodeur propre `explorer-net::consent`, champs relus par
  les re-décodeurs : versions, langue, bitfields
  purpose/vendor), `OptanonConsent`/`OptanonAlertBoxClosed`
  (OneTrust), `notice_behavior` (Didomi),
  `cookieconsent_status` (Cookiebot), `usprivacy` (USP).
- **`AcceptAll`** : outil de diagnostic — maximalise la
  compatibilité au prix d'un consentement déclaré partout.

Le grain est posé **par nom, jamais en écrasant** : un
`Set-Cookie` réel du site (ou un état consenti antérieur
persisté via le profil) reste prioritaire sur la décision de
politique — le moteur ne « re-décide » pas par-dessus un vrai
choix déjà stocké.

Réglage de vol : `EXPLORER_CONSENT_POLICY=honest|reject|accept`
sur le daemon — les agents comparent un même mur sous chaque
politique.

## Honnêteté (bornes assumées)

- `cmpId = 0`, `vendorListVersion = 0` : le moteur n'usurpe
  aucun identifiant IAB — la TC string est lisible mais se
  dénonce comme non-émise par une CMP enregistrée.
- `RejectAll` n'est pas un mensonge : les bitfields disent
  « rien de consenti » — le site débloque son contenu
  fonctionnel et coupe légitimement trackers/pubs, comme
  après un vrai clic « Tout refuser ».
- Les murs « consent or pay » tiennent : refuser tout n'ouvre
  pas un paywall-consent. Verdict de vol distinct
  `consent_wall:hard` — pas une régression du moteur.
- Les formats exotiques (`didomi_token` signé, `sp_`/
  `consentUUID` Sourcepoint encodés) ne sont **pas** forgés —
  une valeur invalide y serait pire qu'une absence.

## Conséquences

- `explorer-net::consent` : `ConsentPolicy`, codec TC string
  v2 (segment core, bitfields + ranges + restrictions
  éditeur, round-trip testé), `seed_headers`.
- `explorer-core` : `SessionConfig::consent_policy`
  (défaut `RejectAll`), `seed_consent_cookies` appelé dans
  `nav_begin` — le `Cookie:` de la requête top-level et la vue
  `document.cookie` portent la décision dès le premier hop.
- **Livré (JS-C259)** : `__tcfapi` résolveur formé IAB v2
  piloté par la session — `CmpConsent`/`CmpPolicy` autonomes
  dans `explorer-jscore` (aucune dépendance interne),
  traversée `JsEngine::set_cmp_consent` → `Cmd::SetCmpConsent`
  → `Engine::set_cmp_consent`, posée avant `install_dom`.
  `tcData` complet (`tcloaded`, `listenerId` croissant,
  bitfields purposes/vendors de la politique) ;
  `tcData.tcString` = l'`euconsent-v2` **relu du jar**
  (`cmp_consent` dans `js_glue`) — `document.cookie` et
  l'API servent la même chaîne. APIs sœurs bornées aux
  call-shapes du corpus : `__cmp` (v1), `__uspapi`, `__gpp`,
  `__sdcmpapi` (Sirdata), `OnetrustActiveGroups`/
  `OptanonWrapper` (setter → appel immédiat),
  `Didomi`/`didomiState`/`didomiOnReady` (`push` intercepté),
  `_sp_`, `Osano`, `__tcfapiLocator` (marqueur — le pont
  `postMessage` `__tcfapiCall` reste un trou documenté).
  Toutes les props sont posées `FIXED` (non-rewritable,
  non-configurable) : ni assignation ni
  `Object.defineProperty` ni `didomiOnReady = []` ne
  remplacent le résolveur — résilience aux redéfinitions.
  L'état `Engine::cmp` est global au moteur : les realms
  enfants (`install_dom_ctx`) héritent la même décision.
  `Honest` conserve le stub CMP-less JS-C213 inchangé.
- Travaux suivants (même ADR, étapes dédiées) : réarmement
  des balises `text/plain` marquées CMP sous politique
  résolue (`optanon-category`, `data-cookieconsent`,
  `data-purpose`, `data-src` — le bras dominant du « meurent
  avant `<video>` ») ; substitution de SDKs CMP
  (`consent_stub_for`) au cas par cas uniquement — chaque
  hôte substitué est une entrée de table documentée, pas une
  émulation générique ; tag `cmp=<vendor>` dans le WARN
  `evals_sans_mutation` + re-vols terrain du corpus EU.
