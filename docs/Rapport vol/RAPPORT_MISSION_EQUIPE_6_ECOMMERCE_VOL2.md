# 🛰️ RAPPORT D'ANALYSE EN VOL — ÉQUIPE 6 (E-COMMERCE) — VOL 2

**Complément du premier vol** (`RAPPORT_MISSION_EQUIPE_6_ECOMMERCE.md`,
amazon/ebay/leboncoin/ikea). Ce second passage cible les quatre
places de marché restantes. **Constat majeur du vol : première
boucle infinie de navigation observée en production réelle** —
cdiscount.com a généré **3 812 navigations complètes** et plus de
**10 000 streams HTTP/2** en ~16 minutes sans jamais produire de
capture, jusqu'à ce que le serveur émette un `GOAWAY` (plafond
10 000 streams atteint) et que le processus soit tué à la main.

---

## 📋 Synthèse des vols

| N° | Date / Heure | URL cible | Statut | Anomalie / Diagnostic clé |
|:---:|:---:|:---|:---:|:---|
| **05** | 2026-10-26 12:43 → 12:59 | `https://www.cdiscount.com/` | 🔴 **BOUCLE INFINIE — processus tué** | Page interstitielle anti-bot servie (2 scripts inline + beacon Cloudflare) → **re-navigation auto-entretenue** (`<meta refresh>` CAP-143 ou `location.*` CAP-152) sans aucun plafond : 3 812 `nav: fetch+parse`, `StreamId(20001)` avant `GOAWAY`, log 33 Mo / 278 018 lignes. Aucune capture. |
| **06** | 2026-10-26 12:59 | `https://www.etsy.com/fr/` | 🔴 BLOQUÉ EXTERNE | **HTTP 403** shot et dom (~90 ms, fenêtre H2 `size_increment: 16711681` — edge Etsy). Corps d'erreur quand même cascade+layout+repaint (98 ms layout perdu). Aucun PNG. |
| **07** | 2026-10-26 12:59 | `https://www.zalando.fr/` | 🟠 **SNIFé → `/supported-browsers`** | Le serveur a détecté le navigateur et servi la page « navigateur non supporté » (`ern:static::supported-browsers`, Mosaic/`enrichedRootEntity`, capteurs Akamai `/akam/13/` + scripts offusqués `/ohF_PSG5/…`). PNG 4,46 Mo produit. 2 TypeError jscore dans les capteurs Akamai. |
| **08** | 2026-10-26 13:00 | `https://fr.aliexpress.com/` | 🟠 **PUNI → `///_____tmd_____/punish`** | Redirection en-session vers la page de punition Alibaba (**x5sec**, `sufei-punish/0.1.129`, captcha AWSC, QR code). PNG 1,84 Mo produit. **+ anomalie moteur grave : un script punish a figé jscore 30 s (`moteur figé — éval dépassée`) puis le moteur est resté `inerte` pour tout le reste du vol.** |

---

## 🔬 Fiches Techniques Détaillées

### ✈️ Vol #05 — `https://www.cdiscount.com/` — ⚠️ ANOMALIE MAJEURE

- **Log** : `logs/eq6_www.cdiscount.com.log` — **33 Mo, 278 018 lignes**,
  12:43:26 → 12:59:42 (~16 min), processus terminé manuellement.
- **Rendu visuel** : **aucun** — le vol n'a jamais atteint l'état final.

#### 📈 Métriques de la boucle (extraites du log)

| Métrique | Valeur |
|:---|:---:|
| Navigations complètes (`nav: fetch+parse…`) | **3 812** |
| Scripts émis (tous = `beacon.min.js` Cloudflare) | 3 812 |
| Évals script `ok=true` (2 inline : 2 259 + 921 octets) | ~7 600 |
| `drain dyn links n=0` | 34 772 |
| StreamId max atteint (connexion n°1) | **StreamId(20001)** |
| `GOAWAY` serveur (`NO_ERROR, last_stream_id=19999`) | 12:54:09 |
| Reconnexion H2 + poursuite de la boucle | oui (StreamId repart à 1, jusqu'à 10 487+) |
| Erreur JS répétée | `TypeError: cannot read 'appendChild' of Undefined at c (1:13)` ×3 811 |
| Cadence | ~4 navigations/s, ~150–300 ms par cycle |

#### 🔁 Mécanisme de la boucle (analyse racine)

Chaque cycle est **une navigation complète**, pas une sous-ressource :

```text
nav: fetch+parse… tab=1
  → fetch+parse ok → peinture ms=36-143        (document minuscule)
  → script émis …/beacon.min.js  module=true
  → sous-ressources ok → commit
  → eval 2 259 o + eval 921 o + module beacon  (tous ok=true)
  → apply cascade/boxtree/layout/repaint (~50 ms)
  → listener dom en échec ty="DOMContentLoaded"
      TypeError: cannot read `appendChild` of Undefined  at c (1:13)
  → drain dyn links n=0  →  nav: fetch+parse…  (cycle suivant)
```

Le document servi par l'edge cdiscount est une **page interstitielle**
(probable dépose de cookie/challenge CDN — Akamai/`WindowUpdate
2147418112`, même signature que Datadome au vol 1) qui **s'auto-re-navigue**.
Deux coutures candidates dans `explorer-core`, toutes deux non bornées :

1. **`<meta http-equiv="refresh" content="0">` (CAP-143)** : chaque commit
   ré-arme `entry.refresh_at` (`session/navigate.rs:1038-1043`) ; le tick
   `pump_timers` (`session/js_glue.rs:30-44`) le transforme en
   `DrainAct::Req` drainé par `drain_script_nav` — **sans compteur :
   chaque nouveau document peut armer une nouvelle échéance à
   l'infini**.
2. **`location.reload()` / `location.href` (CAP-152)** depuis le script
   inline de 921 octets : `drain_script_nav` borne à
   `MAX_NAV_HOPS = 8` intentions **par drain**
   (`session/navigate.rs`, `const MAX_NAV_HOPS: u8 = 8`) — mais la borne
   est par-appel, pas par session : une intention par tick suffit à
   boucler indéfiniment.

Le log ne tranche pas entre les deux (aucune trace de la cible de
navigation émise) — **les deux chemins partagent le même défaut : aucun
budget transversal de navigations automatiques**.

#### 💥 Anomalies moteur identifiées (`explorer-core` / `explorer-net` / `explorer-jscore`)

```text
A. Absence de budget de navigations automatiques :
   - pas de plafond « navigations auto par onglet / par session » ;
   - pas de détection de boucle (même URL, même document, N commits
     identiques consécutifs) ;
   - pas de timeout global de vol côté CLI/daemon : le shot n'est
     jamais arrivé et rien n'a interrompu la session en ~16 min.

B. Absence de plafond de sous-ressources : `plan_subresources`
   (session/net.rs:2749) et `spawn_dyn_subresources` (net.rs:648)
   n'ont pas de borne visible par page ; `drain dyn links n=0` est
   pompé 34 772 fois (poll en boucle à chaque point de synchro).

C. `GOAWAY` serveur absorbé silencieusement : à StreamId(19999)
   cdiscount ferme la connexion (NO_ERROR — son plafond de 10 000
   streams/connexion) ; `explorer-net` re-bind une connexion et la
   boucle repart — correct côté tolérance, mais aucun garde-fou ne
   remarque qu'une page a épuisé 10 000 streams.

D. `TypeError: cannot read 'appendChild' of Undefined at c (1:13)`
   ×3 811 : le listener `DOMContentLoaded` de l'interstitiel appelle
   `X.appendChild` sur `Undefined` — probablement `document.body` ou
   `document.head` non exposé / Undefined dans le DOM jscore au moment
   de l'événement, ou getter manquant. Erreur tolérée (script en
   échec, pas de crash) mais signature à investiguer — c'est
   possiblement elle qui empêche le challenge de se résoudre, donc
   la cause *déclenchante* côté page du re-load permanent.
```

#### 🧪 Repro minimal proposé

```html
<!-- fixture : page qui s'auto-rafraîchit — doit s'arrêter seule -->
<!doctype html><meta http-equiv="refresh" content="0;url=/self">
<script>document.addEventListener('DOMContentLoaded',()=>{});</script>
```

```rust
// Attendu : un compteur `auto_nav_count` par onglet (config,
// ex. 10-20) → au-delà, la navigation est refusée, l'entrée reste
// figée, warn! « boucle de navigation auto détectée ».
// Idem pour location.reload() répété sur la même URL.
```

Critères de fix suggérés (NAVIGATEUR, étape dédiée) :
- `SessionConfig::max_auto_navigations` (compteur cumulé meta-refresh +
  `location.*` + `history.go(0)` par onglet, reset sur navigation
  utilisateur) ;
- détection de boucle stricte : même URL + même corps → stop après
  N=2 cycles identiques ;
- timeout global de `navigate`/`shot` côté `explorer-cli`/`daemon` ;
- plafond `max_subresources_per_page` + `max_streams_per_page` dans
  `plan_subresources`/`spawn_dyn_subresources`.

---

### ✈️ Vol #06 — `https://www.etsy.com/fr/` (anti-bot edge)

- **Log** : `logs/eq6_www.etsy.com.log` (37 lignes — vol le plus court).
- **Rendu visuel** : **aucun**.

#### 💥 Mur détecté (externe) :

```text
DEBUG explorer_core::session::paint: pipeline en échec tab=1 error=HTTP 403
erreur : HTTP 403
```

Headers reçus ~35 ms après la requête, `WindowUpdate 16 711 681`
(edge Etsy). Comme eBay/leboncoin au vol 1 : le corps 403 est
loyalement cascade/layout/repaint (98 ms de layout jetés) avant que
l'erreur ne remonte.

**Signature de détection (cumulée)** : `User-Agent: Explorer/0.1`
(`crates/explorer-net/src/h1.rs`, propagé H2), absence de `sec-ch-ua`/
`sec-fetch-*`/`Accept-Language`, fingerprint TLS rustls, SETTINGS H2
par défaut. Etsy n'a même pas servi de challenge — refus sec.

---

### ✈️ Vol #07 — `https://www.zalando.fr/` (sniffé → supported-browsers)

- **Log** : `logs/eq6_www.zalando.fr.log` (6 699 lignes, DOM dump inclus).
- **Rendu visuel** : `captures/2026-10-26_13-59-50_Equipe_6_Ecommerce_
  www.zalando.fr.png` (**4,46 Mo**) — la page « navigateur non
  supporté », rendue proprement.

#### 🔬 Ce qui s'est passé

Le serveur Zalando a **reniflé le navigateur et routé vers
`/supported-browsers`** — visible dans le DOM dump :

```text
"uri":"https://www.zalando.fr/supported-browsers"
"enrichedEntity":{"id":"ern:static::supported-browsers","type":"static"}
require(["…/rnd/ng/static_supported-browsers/index-008ff2….js"])
```

Page servie par le moteur **Mosaic** (Zalando) : graphes
`enrichedEntity`/`graphqlCache` embarqués en `<script type=
"application/json" class="re-data-el-hydrate">`, bundle `runtime-
8f8e4f0a…` + `hydrate(reRootNode)`, header/navigation/traductions
GraphQL complets (le SSR de la page d'erreur est *riche* — d'où les
4,46 Mo de PNG).

#### 🕵️ Signature de détection

- Capteurs **Akamai Bot Manager** présents dans la page :
  `<script src="/akam/13/579310c7">`, pixel noscript
  `/akam/13/pixel_579310c0?a=…`, scripts offusqués à chemins rotatifs
  `/ohF_PSG5/…` et `/I33j9H/…` (challenge sensor bmak).
- Le reroutage est vraisemblablement côté edge (en-têtes/UA/TLS) —
  pas de cookie préalable possible.

#### 💥 Erreurs `explorer-jscore` (dans les capteurs Akamai, non bloquantes) :

```text
TypeError: `r` is not a function (on Object)
  at F7 (…/ohF_PSG5/…/DBRJbg/4sI1s:1:23036) …
TypeError: cannot read `name` of Undefined
  at gK (…/ohF_PSG5/…/JzBgwqAQ?v=f431f4b1-…:1:480792)
  at qQ (…/I33j9H/…/RfZl8:1:538847) …
```

Le code obfusqué Akamai (vm bytecode + détection d'environnement)
plante sur deux manques probables : un appelable non exposé
(`r` — possiblement un getter API manquant) et une propriété `name`
d'objet Undefined (probablement `navigator.*` ou une cible de feature-
detection). Toléré sans crash. Point positif : les scripts du DOM
de la page, Sentry, RequireJS (`require-2.1.22.min.js`) et les
dyn-links Mosaic (`seo/glados/…-main.css` fetché en dyn link)
fonctionnent.

#### 🔍 Analyse racine : **mur externe**, moteur recevant. La page
d'erreur étant du vrai HTML riche, le pipeline a produit un beau
rendu — le bug est « amont » : Explorer est détecté. Mêmes
contre-mesures documentées qu'au vol 1 (UA, empreinte H2/TLS).

---

### ✈️ Vol #08 — `https://fr.aliexpress.com/` (punition x5sec)

- **Log** : `logs/eq6_fr.aliexpress.com.log` (882 lignes).
- **Rendu visuel** : `captures/2026-10-26_14-00-38_…_fr.aliexpress.
  com.png` (**1,84 Mo**) — page de punition rendue.

#### 🔬 Ce qui s'est passé

Navigation initiale servie en 139 ms (stub), puis **re-navigation
automatique immédiate** (+23 ms) vers
`fr.aliexpress.com///_____tmd_____/punish?x5secdata=…` — le dispositif
anti-bot **x5sec** d'Alibaba. La page punish charge :

```text
g.alicdn.com/mtb/lib-flexible/0.3.2/flexible.js
g.alicdn.com/sd/punish/0.0.1/qrcode.min.js
g.alicdn.com/mtb/??lib-promise/3.1.3/polyfillB.js,lib-mtop/…/mtop.js,lib-windvane/…
g.alicdn.com/AWSC/CAPTCHA/0.0.1/awsc.js        ← captcha Alibaba
g.alicdn.com/bsop-static/sufei-punish/0.1.129/build/htmltocanvas.min.js
g.alicdn.com/bsop-static/sufei-punish/0.1.129/build/punishpage.min.js
g.alicdn.com/trace/trace/1.3.22/??sdk.js,…    ← télémétrie punish
```

(Nota : le combo-URL `??a.js,b.js` d'alicdn est correctement fetché —
un seul stream agrégé, traité.)

#### 💥 Anomalie moteur grave (`explorer-jscore`) — **gèle puis inerte** :

```text
13:00:40.19  sous-ressource …/trace/1.3.22/??sdk.js…  (fetch ~30 s !)
13:01:10.20  script async en échec : moteur figé — éval dépassée
13:01:10.20  h2: connexion terminée — broken pipe (la fenêtre de 30 s
             a fait mourir la connexion alicdn entre-temps)
13:01:40.37  script eval fin ms=30004 ok=false → « moteur figé »
13:01:59.71  script eval fin ms=19339 ok=false
13:01:59.71+ tous les scripts/listeners suivants :
             « moteur js inerte » (readystatechange, DOMContentLoaded,
             load — tous en échec)
```

Deux constats :

1. **Le budget d'éval existe (30 s) mais est bloquant** : pendant
   30 secondes le fil JS est figé — la phase sous-ressources a duré
   **31 873 ms** au commit, et la connexion H2 alicdn est morte de
   broken pipe pendant l'attente (corollaire : pas de timeout de
   fetch de sous-ressource distinct, le SDK trace a mis ~30 s à
   répondre).
2. **Le gel est définitif** : après « moteur figé », tout le reste du
   document tombe en « moteur js inerte » — les listeners de la page
   punish (qui posent le cookie `x5sec` de sortie de challenge) n'ont
   jamais tourné. Tolérant (pas de crash, PNG produit), mais le
   challenge ne peut jamais se résoudre.

Le script coupable est un des bundles `sufei-punish`/`awsc` (boucle
d'attente ou crypto synchrone lourde type proof-of-work — classique
des challenges anti-bot). Repro : exécuter `punishpage.min.js`
0.1.129 sous jscore avec instrumentation du point de blocage.

---

## 🧭 Croisement des anomalies (vol 2)

| Anomalie | Crate | Vol(s) | Gravité |
|:---|:---|:---:|:---:|
| **Boucle infinie de navigations auto** — aucun budget (meta refresh / `location.*` cumulés) | `explorer-core` | cdiscount (3 812 navs) | 🔴 **critique** — vol jamais terminé, DoS auto-infligé |
| Absence de plafond sous-ressources / streams par page | `explorer-core` (`plan_subresources`, `spawn_dyn_subresources`) | cdiscount (10k+ streams) | 🔴 |
| Pas de timeout global de navigation/vol | `explorer-cli`/`explorer-core` | cdiscount (~16 min) | 🔴 |
| `document.body`/`head` (ou getter DOM) `Undefined` dans listener DOMContentLoaded → `appendChild` plante | `explorer-jscore` / glue DOM | cdiscount ×3 811 | 🟠 possible déclencheur de la boucle |
| Éval JS 30 s bloquante → « moteur figé » **définitif** ; connexion H2 morte pendant le gel | `explorer-jscore` (+ `explorer-net`) | aliexpress | 🟠 challenge insolvable, moteur perdu pour la page |
| Fetch sous-ressource ~30 s sans timeout dédié | `explorer-core`/`explorer-net` | aliexpress | 🟠 |
| Layout/repaint sur corps d'erreur 403 (déjà noté vol 1) | `explorer-core` | etsy | ⚪ |
| Redirections/snf anti-bot : 403 sec, `supported-browsers`, `punish?x5sec` | **externe** | etsy, zalando, aliexpress, (cdiscount) | ⚪ hors moteur — signatures documentées |

**Signatures anti-bot rencontrées à ce jour** (catalogue) :
Akamai Bot Manager (`/akam/13/*`, sensor obfusqué, `bmak`) —
cdiscount, zalando ; Datadome (challenge cookie `datadome`) —
leboncoin ; edge sec 403 — ebay, etsy ; **x5sec Alibaba**
(`///_____tmd_____/punish`, `sufei-punish`, captcha AWSC) —
aliexpress. Déclencheurs communs : `UA: Explorer/0.1`, absence
`sec-ch-ua`/`sec-fetch-*`, TLS rustls, comportement de navigation.

---

## 🧭 Verdict d'équipe (vol 2)

Le vol 2 fait basculer le diagnostic : les murs anti-bot attendus
sont bien là (4/4 sites filtrés, avec trois mécanismes distincts),
mais **l'anomalie n°1 est interne** : cdiscount démontre qu'une page
auto-re-navigante — pattern banal des interstitiels CDN — fait tourner
`explorer-core` en boucle sans garde-fou : aucun plafond de
navigations automatiques, aucun plafond de sous-ressources, aucun
timeout de vol. `MAX_NAV_HOPS = 8` borne un drain, pas une session.

**Priorités proposées (NAVIGATEUR)** :
1. Budget de navigations automatiques par onglet (CAP-143/CAP-152
   cumulés) + détection de boucle même-URL — **avant tout nouveau
   vol sur sites à interstitiels**.
2. Timeout global de `navigate`/`shot` et plafond de
   sous-ressources/streams par page.
3. `explorer-jscore` : exposer `document.body`/`document.head` au
   dispatch `DOMContentLoaded` (verrou probable du challenge
   cdiscount) ; rendre le « moteur figé » récupérable ou au moins
   borner le temps bloqué en dessous du timeout réseau des
   sous-ressources.
