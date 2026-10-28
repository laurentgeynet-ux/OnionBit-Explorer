<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# Rapport de vol — Agent 107 : PRESSE2 `[V10-main]`

- **Binaire** : `[V10-main]` —
  `C:/vol100-wt/target/release/explorer-daemon.exe` (HEAD
  `400eaf7d`, aucun nouveau fix depuis JS-C182 — binaire
  frais conservé).
- **Mission** : PRESSE2 — cibles reportées du vol 106 :
  franceinfo, deezer (+ github reporté §B-59).
- **Daemon** : PID **3828**, port 9640, **watchdog actif**
  (seuil 5 120 Mo — jamais atteint : pic 1 976 Mo, retour
  807 Mo) ; journal `_vol107_daemon.log` + `_vol107_watch.log` ;
  arrêt propre, DELETE propres ×2.
- **Verdict global** : vol calme sous harnais — franceinfo
  ~70 % (menu compte peint ouvert → §B-17, article + photo),
  deezer ~60 % (landing rendue, erreurs nommées) ; §B-14 +3
  (`onReadyStateChange`/`onPrimary`/`filter` sur fn —
  franceinfo, survicate deezer). RAM max 1,98 Go sans
  dérive — le seuil 5 Go n'a pas été approché.

## Synthèse

| Cible | Verdict | Console | Anomalies |
|---|---|---|---|
| franceinfo.fr | **~70 %** — nav/accueil/articles+photo peints ; panneau compte « se connecter » peint **ouvert** | 21 entrées nommées : `writeEmbed is not defined` iframe ×2, `onPrimary of Undefined` ×2, `onReadyStateChange is not a function (on Promise)` ×N | **§B-17** (overlay JS-closable) + **§B-14** (`Promise.onReadyStateChange` — thenable custom ?) ; doc_h=98 233 clamped |
| deezer.com | **~60 %** — landing rendue (offre, hero) | `name of Undefined` ×2 (inline+Akamai), `settings of Undefined` survicate, `filter is not a function (on function)` ×2 | **§B-14 +2** ; doc_h=7 735 |

## Watchdog — fonctionnement nominal

```
pid=3828 : 1 537 → 1 976 → … → 807 Mo (pics nav puis purge)
seuil 5 120 Mo non approché ; kill inutile — surveillance seule
```

## Arrêt

DELETE propres ×2 ; `Stop-Process -Id 3828` ; RAM saine.

## Re-vols reportés au vol 108

- **github** → §B-59 fuite nav (watchdog posé — retenter avec
  `/mem` en direct si un fix atterrit).
- flixbus → §B-54 vie-onglet ; vimeo → §B-1-variante.
- cryptpad/bookshop/caniuse → §B-23/§B-12/§B-57-chemin-3.
- uMap/mastodon/tumblr/wttj → §B-28/22/19/36.
