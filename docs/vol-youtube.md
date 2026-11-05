<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Vol YouTube — protocole dédié

Spécialisation de [`vol-agent.md`](vol-agent.md) pour la campagne
YouTube (`docs/roadmap_youtube_fix.md`, worktree
`nav/youtube-fix`). **Tout ce qui n'est pas redéfini ici applique
le protocole général** : séquence canonique §0, règles de vol §5,
garde vignette `_view_*`, verdicts `consent_wall:*`.

Deux différences fondamentales :

1. **Dossier dédié** — tous les artefacts YouTube atterrissent
   dans `C:\OnionBit-vol\youtube\`, jamais à la racine du corpus
   (ni dans le repo).
2. **Batterie d'env `EXPLORER_*` obligatoire** — YouTube est la
   cible la plus lourde du corpus ; un vol sans la batterie
   complète produit des constats faux (murs artificiels §B-80/81).

## 0. Dossier dédié — `C:\OnionBit-vol\youtube\`

| Artefact | Chemin canonique |
|---|---|
| Rapports | `youtube\rapports\RAPPORT_VOL_<date>_AGENT<n>_<TAG>.md` |
| Captures | `youtube\captures\vol<n>_youtube_<cible>.png` (+ `_view_`) |
| Logs & sondes | `youtube\logs\_vol<n>_*.log`, `daemon-*.log`, dumps `eval`/JSON |
| Profils froids | `youtube\prof-vol<n>\` (`cache/`/`session-store/` exclus du git) |
| Binaires de sonde | `youtube\logs\seg_*.bin`, fixtures arrachées au vol |

**Le canon reste à la racine** : `SYNTHESE.md` (§A/§B/§E-bis),
`SYNTHESE_JOURNAL.md`, `CLAIMS.md` ne bougent pas — `youtube/`
cloisonne les artefacts, il ne crée pas un second corpus. Les
lignes §B (73, 94, 95, 97, 98, 99, 41-porteurs…) se mettent à
jour dans `SYNTHESE.md` comme pour tout vol.

Nommage : le préfixe `RAPPORT_VOL_` est conservé (grep corpus
uniforme) — c'est le dossier qui cloisonne, pas le nom. TAG
usuel : `YOUTUBE`, `YTHOME`, `YTWATCH`, `YTEMBED`, `YTREVALID`.

## 1. Lancement — batterie YouTube obligatoire

```powershell
# 1.1 arbre propre + HEAD connu (§0 vol-agent)
git rev-parse HEAD; git status --short

# 1.2 binaire frais — worktree dédié, target-dir dédié
$env:CARGO_TARGET_DIR = "D:/cargo-target/OninionBit-Explorer-YoutubeFix"
cargo build --release -p explorer-daemon   # attendre « Finished »

# 1.3 env du vol — TOUTES obligatoires sauf mention contraire
$env:EXPLORER_VIEWPORT='1366x900'
$env:EXPLORER_MEDIA_CODECS='avc1;hev1;hvc1;vp09;av01;vp08;mp4a;mp3;opus;vorbis;flac;ac3;ipcm'
$env:EXPLORER_JS_OPS='20M'            # §B-81 : bundle polymer 12,6 Mo
$env:EXPLORER_JS_MEM_MIB='384'        # §B-81 : heap borné, dégradation propre
$env:EXPLORER_MEM_DIAG='1'            # muts/heap/dom_nodes par onglet
$env:EXPLORER_PROFILE='C:\OnionBit-vol\youtube\prof-vol<N>'   # froid = défaut
# optionnel — sonde §B-59 (traçage des retraits/remplacements DOM) :
# $env:EXPLORER_DOM_KILL='1'

# 1.4 daemon dédié — plage 9460+ réservée aux vols YouTube
pwsh -File scripts/spawn_daemon.ps1 -Port 946<N> `
  -LogDir "C:\OnionBit-vol\youtube\logs"

# 1.5 watchdogs OBLIGATOIRES — PID = « daemon PID <n> » du spawn
pwsh -File scripts/vol_joblimit.ps1 -TargetPid <pid> -LimitMB 5120
pwsh -File scripts/vol_memwatch.ps1 -DaemonPid <pid> -MaxMB 5120 `
  -Log "C:\OnionBit-vol\youtube\logs\_vol<N>_watch.log" &
```

Pourquoi chaque variable est exigée :

- **`EXPLORER_MEDIA_CODECS`** — sans elle `isTypeSupported` rend
  `""` à froid → `MediaSource` jamais créée → aucune surface à
  claimer (poulet-et-œuf §B-80). Verdicts attendus : conteneur
  connu → `"maybe"`, codecs complets déclarés → `"probably"`
  (§B-94/§B-95 = les deux trous YouTube mesurés sur cette
  batterie : alias `vp9`/`h264`/`av1` → `""`, plafond `maybe`).
- **`EXPLORER_JS_OPS`/`EXPLORER_JS_MEM_MIB`** — contournement
  §B-81 validé en routine : l'explosion non bornée devient
  dégradation propre (le moteur meurt sous bornes **sans** tuer
  le daemon).
- **`EXPLORER_MEM_DIAG=1`** — la télémétrie `muts`/`js_heap`/
  `media_n`/`pending_events` est la signature §B-73/§B-97 : sans
  elle, un squelette figé et un feed hydraté se ressemblent.
- **`EXPLORER_CONSENT_POLICY`** — à déclarer dans le rapport ;
  défaut `reject`. YouTube sert un consent-wall dur côté EU :
  distinguer `consent_wall:soft` (régression) de `hard` (modèle
  du site, pas un bug) — profil **froid** exigé.
- **`EXPLORER_DOM_KILL`** — sonde de bissection §B-59 uniquement ;
  la laisser posée sur un vol de mesure fausse les constats DOM.

## 2. Cibles canoniques

Vidéo de référence : **`jNQXAC9IVRw`** (« Me at the zoo » —
stable, courte, première vidéo YouTube).

| Cible | Ce qu'elle éprouve | §B |
|---|---|---|
| `https://www.youtube.com` | Hydratation Polymer/ShadyDOM/PolySi — « tout voir » | §B-73 → §B-97 (6 racines) |
| `https://www.youtube.com/watch?v=jNQXAC9IVRw` | `html5player` `base.js` + SABR/UMP + MSE — « lire » | §B-98, §B-99, §B-94/95 |
| `https://www.youtube.com/embed/jNQXAC9IVRw` | Embed en nav directe | erreur 153 **attendue** |
| `<iframe src="…/embed/jNQXAC9IVRw">` sur page tierce | Scripts externes du sous-cadre + `Referer` | §B-99 (bras embed, Étape 207) |

**Erreur 153 en nav directe `/embed/`** : YouTube exige un
`Referer` tiers pour servir un embed — `embedder.identity.
missing.referrer` peint l'écran d'erreur **correctement**. C'est
le comportement du site, pas une régression ; le verdict utile
est « l'écran d'erreur se peint ». La vraie mesure embed se
fait en iframe sur page hôte (`python -m http.server` suffit).

## 3. Mesures obligatoires par cible

Un rapport YouTube qui ne donne pas ces chiffres est incomplet —
la signature §B-97/98 vit dans la télémétrie, pas dans le PNG.

### Accueil `/` (« tout voir »)

- `eval` : `!!document.querySelector('ytd-app').shadowRoot` —
  `true` attendu (0 `NotFoundError`, 0 `HierarchyRequestError`
  au `GET …/console`).
- `ytd-rich-grid-renderer` : `containerWidth > 0` et
  `shownItems.length > 0` (tuiles `<ytd-rich-item-renderer>`
  peuplées — titres + miniatures visibles sur la frame).
- `/mem` : `dom_nodes` (> ~2 000 hydraté vs 1 610-1 620
  squelette), `muts`, `js_heap` (> 0 — `heap→0` = moteur mort),
  timers, `pending_events`.
- `/console` : erreurs pipeline de boot (persistantes — premier
  réflexe avant le DOM).
- Frame PNG complète + compte disjoncteur §B-41 (`tempête de
  pompes` au stderr) ; `/frame` qui rend 0 octet = constat en
  soi (§B-73 variante, vol 592).

### Lecture `/watch` (« lire »)

- `eval` : `playabilityStatus.status` — `"OK"` attendu ;
  `adaptiveFormats` présents.
- **Zéro `sabr.malformed_config`** au console/stderr, et **pas
  de redirection** `…&pbj=1` (le fallback JSON-brut = preuve
  que le POST protobuf est mort — §B-98).
- `MediaSource` : attache `src="blob:…"` (`media_n=1` au
  mem_diag), `sourceBuffers.length >= 1`.
- `video.getVideoPlaybackQuality()` → objet non-`undefined`
  (`droppedVideoFrames` lisible — §B-99).
- `GET …/media` : offre(s) MSE vivante(s) ; cycle claim →
  `rest_host_probe` niveau b si la DLL est déployée
  (`frames`/`pcm`/`pos` qui avancent = décodage réel).
- Batterie codecs : verdicts `isTypeSupported` sur
  `codecs="vp9"`/`"h264"`/`"av1"` (alias → §B-94) et sur les
  chaînes complètes (`avc1.42…`, `av01.0.…` — §B-95 `probably`).

### Embed

- Nav directe : écran **Erreur 153** peint = conforme.
- Iframe sur page hôte : `www-embed-player.js`/`base.js`
  fetchés **dans le ctx de l'iframe** (pas `max_src=0` —
  Étape 207), `Referer` émis, player au-delà du spinner.

## 4. Trame de rapport

En-tête minimal (en plus du corps libre) :

```markdown
- **Date** : AAAA-MM-JJ
- **Vol / Agent / Port / PID** : N / N / 94xx / PID
- **Binaire** : <sha> (+ tag [V…])
- **Env** : MEDIA_CODECS=…, JS_OPS=…, JS_MEM_MIB=…, MEM_DIAG=…,
  DOM_KILL=…, CONSENT_POLICY=…
- **Profil** : froid `youtube\prof-vol<N>` | gabarit | chaud
- **Cibles / Captures / Journaux** : chemins youtube\…
```

Chaque constat porte son **ID §B** (occurrence → incrémenter la
ligne `vol N : site ×N` dans `SYNTHESE.md` ; trou nouveau →
proposition §B au numéro suivant). Verdicts consentement sous
`consent_wall:soft|hard` avec la politique du vol.

## 5. Clôture

1. `DELETE /api/tabs/{id}` puis kill daemon **par PID**.
2. `git add -A && git commit` dans `C:\OnionBit-vol` — couvre
   `youtube/` comme le reste du canon.
3. Entrée `SYNTHESE_JOURNAL.md` + statuts §B synchronisés à la
   racine ; ligne `CLAIMS.md` libérée.
