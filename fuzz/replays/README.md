# `fuzz/replays/` — fixtures `DirTransport` versionnées

Corpus de rejeux réseau **trackés dans git** (convention vol §8 :
le corpus brut des vols vit sous `Explorer-data/vols/`, les
rejeux **promus** sont versionnés ici pour rester rejouables
hors-ligne et servir de filet de régression).

`fuzz/corpus/` et `fuzz/artifacts/` restent les dossiers
locaux de `cargo fuzz` — non suivis, rien à voir.

## `youtube-vol813/`

- **Provenance** : vol 813 (enregistrement `EXPLORER_NET_RECORD`
  complet, 67 Mio sous `Explorer-data/vols/youtube/replay-vol813/`),
  **distillé** au sous-ensemble réellement servi à un shot borné —
  chaque `.resp`/`.req` de ce dossier a été consommé par le moteur.
- **Référence Chrome embarquée** : `www.youtube.com.ref.json` /
  `.ref.png` (copie de `Explorer-data/vols/youtube/refs/`,
  capturée par `scripts/ref_capture.mjs`) — le shot de contrôle
  est 100 % hors-ligne.
- **Verrou** : `explorer-net::replay::tests::
  dir_transport_replays_youtube_flight` — chaque `.resp`
  doit décoder, le document `https://www.youtube.com/` doit
  être servi.

## Utilisation

```powershell
# Shot moteur sur le rejeu (aucun socket ouvert) :
$env:EXPLORER_NET_REPLAY = 'fuzz\replays\youtube-vol813'
explorer-cli shot https://www.youtube.com/ out.png --geometry out.geom.json

# Ou le shot de contrôle complet (shot + geo_diff vs référence) :
pwsh -File scripts/vitrine_shot.ps1
```

## Régénération / promotion d'un autre vol

La distillation ré-enregistre **pendant** le rejeu — seules
les réponses que le shot consomme sont écrites (étape 313 :
`EXPLORER_NET_RECORD` enveloppe `EXPLORER_NET_REPLAY`) :

```powershell
$env:EXPLORER_NET_REPLAY = 'Explorer-data\vols\youtube\replay-vol813'
$env:EXPLORER_NET_RECORD  = 'fuzz\replays\youtube-vol813'
explorer-cli shot https://www.youtube.com/ _tmp.png --geometry _tmp.geom.json
```

Puis vérifier la déterminisme : rejouer le même shot sur la
fixture distillée — la `geom.json` doit être identique.
