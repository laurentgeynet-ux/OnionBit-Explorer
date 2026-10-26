# ADR-0024 — Lecture média : responsabilité d'hôte, seam `MediaSurface`

Statut : **Acceptée** — livrée (étape 71a–d), amendée
2026-10-25 (stack hôte nommée, § Amendement). Prolonge
ADR-0023.

## Contexte

ADR-0023 a figé le contrat interne : stubs honnêtes +
affiche, aucun décodeur dans le workspace (règle §2 — pur
Rust, `#![forbid(unsafe_code)]`). Cette décision tient et
n'est pas revisitée : le moteur ne décode pas.

La question restante est le **service** : un navigateur
qui ne lit jamais aucun média ne sert pas le web réel.
Trois constats structurent la réponse :

1. **Le modèle industriel est déjà « hôte ».** Android
   WebView, WPE, Servo embarqué : le moteur expose une
   région + une source, le lecteur de la plateforme fait
   la reste (codecs OS, pipeline matériel, overlays
   composites). Le moteur n'a jamais contenu les codecs
   — il contient le contrat.
2. **La couverture interne pure Rust est bornée et
   coûteuse.** `symphonia` (audio, `#![forbid(unsafe)]`,
   MP3/AAC/Opus/Vorbis/FLAC) et `rav1d` (AV1, BSD-2,
   asm interne admissible §4.1) existent — mais H.264
   (dominant), VP9 et HEVC n'ont aucun équivalent pur
   Rust viable ; les porter = années-homme. La pile
   interne couvrirait un sous-ensemble réel, jamais le
   tout.
3. **L'hôte Flutter de ce projet sait déjà jouer.** Le
   pattern `AbstractPlayerEngine` → backends plateforme
   (`video_player`/`fvp` sous FFmpeg desktop, ExoPlayer
   sous Android) est éprouvé dans l'écosystème adjacent —
   couverture = tout ce que FFmpeg/ExoPlayer lisent, plus
   le HLS natif des players OS.

Les deux murs restants, indépendants de tout choix :

- **DRM** (Netflix/Disney+/Prime : Widevine/FairPlay) —
  licence contractuelle du CDM, blob fermé certifié par
  produit ; aucun moteur indépendant n'y accède. Hors
  portée **structurellement**, pas techniquement.
- **MSE** (YouTube/Twitch : lecture pilotée par JS via
  `MediaSource`/`SourceBuffer.appendBuffer`) — exige une
  API JS complète + remontée des buffers vers le lecteur
  hôte. Hors périmètre v1 ; le seam ci-dessous le
  permettra sans le requérir.

## Décision

**La lecture est un service d'hôte contracté par un seam,
pas une capacité du moteur.**

1. **`MediaSurface`** — nouveau contrat de couche
   interface : le moteur émet un événement SSE
   (`MediaOffered` : `tab`, `node`, `rect` de la boîte
   remplacée, `url` résolue, `mime`, `controls`) quand un
   élément média devient lisible ; l'hôte qui souhaite
   jouer répond (`POST /api/media/{id}/claim`) puis pilote
   la lecture (`POST …/{play|pause|seek|volume}`). Le
   moteur peint toujours : poster, contrôles, box de
   repli — l'hôte composite sa surface vidéo **au-dessus**
   du rect déclaré.
2. **Surface JS devient vraie par délégation** — le
   contrat honnête d'ADR-0023 est conservé mot pour mot
   quand aucun hôte ne claim la surface (`play()` rejette,
   `canPlayType` = `""`). Quand un hôte est attaché et
   déclare ses codecs, `play()` résout à la première
   frame hôte, `currentTime`/`duration`/`buffered`
   reflètent l'état hôte via events SSE → props JS.
   **Jamais de faux succès** : un hôte muet = stub.
3. **Zéro codec dans le workspace.** Aucune dépendance
   de décodage n'entre ; §2 intact. Les éventuels
   décodeurs internes (`symphonia`, `rav1d`) restent une
   option **additive** ultérieure qui se brancherait au
   même seam (producteur interne vs hôte externe) — la
   décision les rend possibles sans les exiger.
4. **Fallbacks intacts** — host absent, claim refusé,
   lecture échouée → le document garde poster + boîtes
   UA ; la dégradation est le chemin normal, pas un cas
   d'erreur.

## Conséquences

- **Couverture** : tout `<video src>`/`HLS`/audio
  progressif = ce que le player hôte sait lire (sur
  Windows via `fvp`/FFmpeg : H.264/HEVC/VP9/AV1 + tous
  conteneurs — quasi-totalité du web non-DRM). **Vigilance
  licence** : la config de build FFmpeg/libmpv de `fvp`
  doit être vérifiée (LGPL dynamique compatible MIT/
  Apache ; GPL statique non) avant d'y adosser l'app.
- **MSE et DRM documentés hors atteinte** — l'écart est
  nommé, pas découvert au terrain.
- **Le proxy loopback** (patron `local_proxy_server` /
  `TorProxyServer`) devient le point unique où un flux
  média peut recevoir headers/UA custom ou routage —
  réutilisation directe pour les sources à en-têtes
  exigées.
- **Point de cohérence** : `MediaSurface.rect` doit suivre
  le relayout/scroll — l'hôte repositionne son overlay sur
  `FrameInvalidated` portant les rects médias à jour, ou
  le seam émet `MediaMoved`. À préciser dans
  `interface-equipes.md` à l'implémentation.
- **PIPE** : R-34 (médias vrais remplacés boxtree) donne
  au seam une géométrie fiable — taille intrinsèque du
  poster, rect exact — prérequis documenté, non bloquant.

## Amendement 2026-10-25 — stack hôte nommée

Lecture ambiguïté levée (décision lead) : « zéro codec »
vise les **crates `explorer-*`** — le workspace Rust reste
sans décodeur. Côté `app/`, le backend de lecture est
concrètement **`video_player` + `fvp`** (moteur MDK, FFmpeg
en **liaison dynamique** — LGPL compatible MIT/Apache,
vérifié à l'étape 71d ; non statique). L'option « décodeurs
internes » (`symphonia`, `rav1d`, point 3 de la décision)
devient explicitement **non planifiée** : la couverture
FFmpeg hôte la rend sans besoin — elle ne se rouvrirait que
si un contexte sans hôte l'exigeait (embedder headless,
variante producteur interne du même seam). MSE et DRM
restent nommés hors atteinte — DRM dorénavant couvert par
la stratégie de délégation ADR-0027.

## Amendement 2026-10-25 — périmètre et chaînage nommés

Reformulation de la portée (décision lead) : l'ADR couvre
la mise en route de la lecture des **médias non-DRM —
audio et vidéo** — et, dans le même mouvement de
responsabilité d'hôte, la lecture des **médias présents
dans un dossier externe** à l'app (fichiers locaux de
l'utilisateur). Cette deuxième source est **nommée, non
câblée** : la seam ne transporte aujourd'hui que des URLs
réseau ; un dossier média configurable est une extension
ultérieure (le lecteur accepterait une source fichier au
même point d'injection `MediaPlayerFactory` —
`VideoPlayerController.file` au lieu de `networkUrl`).
Aucun travail n'est demandé dessus ici.

### Chaînage concret (état réel)

```
<video>/<audio> lisible
  → moteur : MediaOffered (SSE) — rect, url, mime
  → hôte : POST …/claim {codecs} (AppConfig.mediaCodecs)
  → lecteur : MediaPlayerFactory(url)
        = VideoPlayerController.networkUrl   ← direct aujourd'hui
        ├─ [jambe proxy NON branchée : local_proxy_server
        │    pour en-têtes custom / routage — à câbler si
        │    un flux l'exige]
        └─ fvp → moteur MDK → FFmpeg (liaison dynamique,
             LGPL vérifiée) → surface vidéo / sortie audio
  → overlay au-dessus du rect peint par le moteur
  → état hôte → POST …/state → props JS + events DOM
```

**Câblé** : seam SSE+REST+FFI (71a–e), lecteur réel
`networkUrl` → fvp/MDK/FFmpeg, rapport d'état borné 4 Hz,
repli honnête sans claim.

**Présent, pas branché** : `local_proxy_server`
(`explorer-net`) — point unique prévu pour les flux à
en-têtes exigés ou routage (ADR §Conséquences) ; la
source URL part aujourd'hui directe au lecteur. À câbler
au premier flux qui l'exige.

**Pas requis** : `ffprobe` — MDK/FFmpeg font déjà le
probing ; ne s'introduirait que pour un besoin de
métadonnées en amont du lecteur (nommé, non planifié).

**Hors périmètre confirmé** : MSE et DRM (ADR-0027).

### Référence prouvée : `C:\IptvMediacenter-Rust`

Le chaînage complet existe **déjà câblé et testé** dans
le projet voisin (core headless Rust, mêmes briques) —
source de vérité pour les patterns, pas à réinventer :

| Maillon | Ici (Explorer) | Référence prouvée |
|---|---|---|
| proxy flux | `local_proxy_server` (`explorer-net`, non branché) | `iptv-infra::network::local_proxy` — Axum zéro-copie, headers/UA injectés, **tout** flux de lecture y passe |
| cast | — | `iptv-infra::network::hls_transcoder` (`/hls/v1/`) : **ffprobe → remux\|encode → HLS** (ffmpeg sur PATH) |
| moteur | `fvp` (plugin Flutter, MDK) | `mdk-bridge` FFI + `MdkPlayerEngine` (`iptv-infra::player::mdk_engine`) — même MDK, surface HWND native |
| seam hôte | `MediaSurface`/`MediaPlayer` | `PlayerEnginePort` + `PlayerEngineDelegate`/`PlayerStateSink` |

Le portage éventuel de la jambe proxy (et plus tard du
cast/dossier externe) doit **s'inspirer de cette
implémentation validée** — mêmes pièges déjà traités
(streaming borné, profils réseau par host, reprise
Range).
