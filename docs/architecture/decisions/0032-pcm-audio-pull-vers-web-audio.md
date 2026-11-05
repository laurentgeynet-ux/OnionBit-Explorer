# ADR-0032 — PCM `audio_pull` vers Web Audio : tap dépôt avant toute couture

Date : 2026-11-14 · Statut : **proposée** — Étape 200
(design seul ; implémentation = phases 1-3 ci-dessous, pas
dans l'étape).

## Contexte

L'audio des médias natifs est aujourd'hui rendu **dans la
DLL** en mode `AUDIO_MODE_DEVICE` (cpal — `lib.rs:886`) :
l'app ne voit jamais un échantillon. Conséquences :

- `AudioContext`/`AnalyserNode`/`MediaElementAudioSourceNode`
  côté jscore ne peuvent pas être servis honnêtement — la
  seule donnée sonore est opaque (le device Windows).
- Pas de visualisation, pas de DSP hôte, pas de mixage entre
  surfaces : volume/mute ne sont que des réglages DLL.

La DLL expose pourtant une sortie pull symétrique à
`frame_pull` vidéo : `set_audio_mode(h, 1)` bascule le sink,
`audio_spec` sert `sample_rate`/`channels`/`format` (u8…f64),
`audio_pull(h, buf, cap)` draine des octets interleaved par
trames entières (`NULL/0` = sonde de dispo, `-1` mode inactif,
seek = flush au premier pull — `lib.rs:985`).

## Le constat que le plan n'anticipait pas

**Mode pull = silence si personne ne rejoue.** Le doc-bloc
est explicite : `1` = « PCM drainé — **aucun device ouvert** »
et la bascule « réinstalle le sink ». La DLL ne fait **pas**
de tee `device + pull` : activer `audio_pull` tel quel coupe
le son de l'utilisateur. Un binding naïf serait donc une
régression produit, pas une feature.

De plus l'app Flutter n'a **aucun** sink PCM natif : pas de
plugin audio dans `pubspec`, et en ajouter un (just_audio,
audioplayers — écriture d'un sink WASAPI avec pacing,
resampling, gestion de glitch) est exactement le genre de
dépendance que §4 exige de justifier ici.

## Décision proposée

**Phase 1 — dépôt `perf` d'abord** : ajouter un mode
`AUDIO_MODE_TAP = 2` (« device + copie pull ») dans
`Onion-explorer-media` — le présentateur garde cpal **et**
pousse une copie dans la file bornée d'`audio_pull`. Coût
dépôt faible (fan-out dans le sink), zéro dépendance app,
zéro risque de silence. À spécifier/coder dans
`C:\Onion-explorer-media-wt\perf` — jamais d'écriture directe
sur master (règle AGENTS dépôt).

**Phase 2 — canal PCM REST loopback** : l'app draine
`audio_pull` vers un endpoint `GET /api/tabs/{id}/media/{mid}/
audio` servi par le même loopback que `…/segments` (ADR-0029)
— chunks bornés, cadence par la file DLL (back-pressure
intégrée, `lib.rs:106-108`). Contrat additif : `audio_spec`
exposé en en-tête/`query` initial pour le format.

**Phase 3 — jscore** : `AudioContext` + `AnalyserNode` +
`MediaElementAudioSourceNode` servis par le canal PCM ;
`destination` reste muet **documenté** (le son audible reste
le device de la DLL — le moteur ne rejoue rien). Limite
honnête : les pages exigeant du Web Audio *audible* (jeux)
restent silencieuses tant qu'il n'y a pas de sink moteur —
l'analyse/visualisation, elle, fonctionne.

## Alternatives écartées

- **Mode pull + sink PCM côté app** (WASAPI via plugin audio)
  : zéro changement dépôt mais un sink complet à écrire
  (pacing, SRC, underruns) + dépendance native — plus lourd
  que le tap, moins fiable.
- **`audio_pull` direct depuis Dart** (mode 1) : silence —
  rejeté, voir constat.
- **PCM par SSE** : base64 + latence + pas de back-pressure
  — le canal REST chunké existe déjà pour `…/segments`.

## Conséquences

- Aucune implémentation dans cette étape : verrous et
  bindings attendent le `AUDIO_MODE_TAP` du dépôt.
- Le son des `<audio>`/`<video>` ne change jamais : le mode
  device reste le défaut ; le tap est opt-in par la seule
  activation de la phase 2.
- Métrique de sortie : `AnalyserNode.getByteFrequencyData`
  qui rend des données réelles sur un média natif claimé,
  sans couper l'audio utilisateur.
