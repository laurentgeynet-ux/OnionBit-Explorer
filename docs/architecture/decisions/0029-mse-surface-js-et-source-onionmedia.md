# ADR-0029 — MSE : surface JS réelle + source `ByteSource` OnionMedia

Statut : **Acceptée** — 2026-10-27. Prolonge ADR-0023
(honnêteté média) et ADR-0024 (`MediaSurface`, lecture
déléguée à l'hôte) ; nomme la stack hôte après l'arrivée
d'OnionCodec/OnionMedia — successeurs internes de
`fvp`/ffmpeg dans l'app.

## Contexte

Media Source Extensions est la voie des plateformes vidéo
(YouTube, Twitch, lecteurs DASH/HLS.js embarqués) : le site
pilote la lecture en poussant des segments via
`MediaSource`/`SourceBuffer.appendBuffer`. ADR-0024 a figé
la lecture comme service d'hôte et nommé MSE « hors
périmètre v1 ; le seam le permettra » — ce document fixe
ce « permettra ».

Deux constats structurent le choix :

1. **La surface JS est livrée sans lecture** (JS-C193) :
   `MediaSource`/`SourceBuffer`/`SourceBufferList`/
   `TimeRanges` réels, `EventTarget` par rail `__et`,
   `appendBuffer` comptabilisé-borné (64 Mio,
   `QuotaExceededError`), `URL.createObjectURL(ms)`
   enregistré (`blob:` → `MediaSource`), `readyState`
   `"closed"` tant qu'aucun `<video>` n'attache.
   `isTypeSupported` → `""` tant qu'aucun transport
   n'existe — l'honnêteté pousse les players vers leurs
   fallbacks `<video src>` déjà servis par `MediaSurface`.
2. **La stack hôte a changé de nature.** L'app n'embarque
   plus ffmpeg/`fvp`/mdk mais **OnionMedia FFI** — le
   lecteur QtAV porté Rust (`D:\Projet\OnionMedia`)
   adossé à **OnionCodec** (`D:\Projet\OnionCodec`),
   réécriture FFmpeg pur Rust : démuxeurs MP4/**fMP4**,
   Matroska/WebM, MPEG-TS, Ogg, WAV ; codecs
   H.264/HEVC/VP9/AV1/VP8 et AAC/MP3/Opus/Vorbis/FLAC/PCM.
   OnionCodec exclut explicitement « la plomberie
   navigateur (MSE) » de son périmètre — c'est à Explorer
   de produire la source.

Le mur historique du plan (« ffmpeg tolère-t-il un fMP4
chunked croissant », option A de l'étude préalable)
disparaît : le consommateur n'est plus un lecteur externe
opaque mais un démuxeur que l'on contrôle — exactement
l'usage pour lequel un flux MSE est prévu (init segment
puis media segments dans l'ordre d'append).

## Décision

**Le moteur produit des files de segments bornées ;
OnionMedia les lit par un `ByteSource` pull sur le REST
existant.**

1. **Contrat source = `onioncodec_core::ByteSource`** —
   trait `read(&mut buf) → n` (`0` = EOF), `size()`,
   `position()`, `seek()`. Un `MseSource` par
   `SourceBuffer`, implémenté côté app dans le wrapper
   FFI du mediaplayer :
   - file non vide → octets du segment courant ;
   - file vide et `endOfStream` non appelé → la lecture
     attend (endpoint long-poll) — le `ByteSource`
     bloque comme une socket ;
   - file vide et EOS → `0` → le décodeur draine
     (`NeedMoreData` → `Eof`, leur modèle) ;
   - `size() → None`, `seek() → Unsupported` — source
     **live**, honnête, pas un fichier.
2. **Transport = REST pull sur le daemon** (ADR-0003 —
   l'app ne consomme que l'API, aucun canal nouveau) :
   `GET /api/tabs/{tab}/media/{media}/segments?cursor=N`
   rend le paquet de segments disponibles (bulk —
   amortir les allers-retours `read()`), plus un flag
   `eos` alimenté par `MediaSource.endOfStream()`. Le
   `ByteSource` côté app traduit `read()` → requêtes
   loopback.
3. **Back-pressure sur les deux rails** : cap par
   `SourceBuffer` dans le moteur (64 Mio,
   `QuotaExceededError` — JS-C193) ; consommation au
   rythme du démuxeur (leur `PacketBuffer` borné).
   `remove()` agit sur les octets non encore consommés ;
   l'éviction des trames déjà démuxées relève du
   lecteur — déviation bornée documentée.
4. **Attache spec** : `video.src = blob:` enregistré
   `kind=mse` → `readyState "open"` + `sourceopen` ;
   `revokeObjectURL`/mort du média → `sourceclose`.
   L'offre `MediaSurface` porte un marqueur `mse` — un
   hôte qui ne possède pas la stack refuse le claim
   honnêtement.
5. **Plusieurs `SourceBuffer` = N `MseSource`** — le cas
   A+V de YouTube/Twitch (audio et vidéo en buffers
   séparés) est une composition côté OnionMedia
   (`onionmedia-pipe` : deux `Demuxer` → un `Player`
   sous `Clock` commune), **pas** un remux côté moteur.
   `isTypeSupported` : mime → `CodecId` via le registry
   de probing OnionCodec — table réelle, pas
   `mediaCodecs` textuel.

## Conséquences

- **Le moteur reste sans décodeur ni démuxeur** (ADR-0023
  intact) : il stocke et compte des octets. Toute la
  sémantique conteneur/codec vit chez OnionCodec —
  `SourceBuffer.buffered` reste estimé par
  `timestampOffset`/fenêtres d'append tant qu'aucun
  retour d'horodatage n'existe (approximation
  documentée ; un retour `TimeRanges` réel depuis le
  démuxeur est un ajout ultérieur admis).
- **Palier 4 (A+V) relocalisé** : le mur passe de
  « remux ffmpeg impossible » à « composer deux démuxeurs
  sous une horloge » — un problème `onionmedia-pipe`
  (`PacketBuffer`/`Clock` existants), pas une pièce
  moteur.
- **Palier progressif/HLS inchangé** :
  `onioncodec-net-http`/`net-hls` couvrent `<video src>`
  via `MediaSurface` tel quel.
- **DRM inchangé** : ADR-0027 (WebView2) — MSE ici ne
  concerne que les flux non protégés (pas d'EME).

## Alternatives rejetées

- **Flux HTTP croissant dans `LocalMediaProxy`**
  (`/mse/{media}/stream` servant un fMP4 qui grandit) —
  le bon shape pour un lecteur ffmpeg externe ; avec un
  démuxeur `ByteSource` sous contrôle, c'est un
  intermédiaire inutile qui perd la frontière de
  segments (`remove`, `timestampOffset`, multi-pistes).
- **Démarqueur de boîtes dans le moteur** (walker
  `moov`/`moof`/`mdat` + EBML pour timestamps et pistes)
  — duplique `onioncodec-demux-mp4`/`matroska` dans le
  crate interdit de le faire bien ; à réintroduire
  seulement si `buffered` exact devient un mur terrain
  mesuré.
- **ABI pull FFI `explorer_mse_read(buf_id)`** — un
  canal hors-API, contraire à ADR-0003 ; le REST
  loopback atteint le même résultat sans nouveau contrat
  binaire.

## Références

- ADR-0023 (stubs honnêtes), ADR-0024 (`MediaSurface`),
  ADR-0027 (DRM hors scope).
- `crates/explorer-jscore/src/mse.rs` (JS-C193) ;
  `onioncodec-core::source::ByteSource` ;
  `onionmedia-pipe` (`Player`, `PacketBuffer`, `Clock`).
