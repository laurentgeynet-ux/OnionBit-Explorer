<!-- This file is part of Explorer. Copyright (C) 2026 Laurent Geynet.
     SPDX-License-Identifier: MIT OR Apache-2.0 -->

# EAP v1 — transport et trames

Complément normatif de `eap-1.schema.json` (le schéma décrit les
messages ; ce document décrit le fil).

## Connexion

- Endpoint : `GET /agent` sur le listener loopback du daemon —
  **le même port que l'API UI**, upgrade WS.
- Négociation : `Sec-WebSocket-Protocol: eap/1` — le serveur refuse
  l'upgrade si le client ne propose pas le sous-protocole.
- Première requête obligatoire : `session.open` — toute autre
  méthode avant l'open répond `-32008 unauthorized`.

## Trames

| Trame | Contenu | Direction |
|---|---|---|
| texte | un objet JSON-RPC 2.0 (`id` présent = requête, absent = notification) | bidirectionnel |
| binaire | `[u32 LE id][u32 LE kind][payload]` — `kind` : `1` = `image/png` | serveur → client |

Les trames binaires sont **exclusivement** des réponses à charge
lourde (`frame.get`) corrélées par `id` au lieu d'un `result` JSON :
le client associe la trame binaire à la requête ouverte de même `id`.
Un serveur n'envoie jamais de binaire non sollicité — les événements
poussés sont des notifications texte (`frame.invalidated` porte les
rects, jamais les pixels : le pull reste le modèle, ADR-0004).

## Ordonnancement

- Les réponses à des `id` distincts peuvent arriver dans le
  désordre — ne jamais supposer FIFO entre requêtes.
- Les notifications d'un même onglet arrivent dans l'ordre de
  commit du single-writer ; entre onglets, pas d'ordre garanti.
- Une trame binaire suit toujours sa requête sans qu'aucune autre
  trame binaire **du même `id`** ne s'intercale.

## Session, droits, erreurs

`session.open` accorde un `profile` — le serveur n'accorde jamais
plus que demandé (`input:false` → tout `input.*` répond `-32008`).
Les `ref` expirent en `-32002 stale_ref` sur navigation, re-parse
ou fermeture d'onglet : l'agent re-snapshote, jamais de revalidation
mutuelle des handles.

## Vecteurs

`golden/` contient des échanges fil à fil `{requête, réponse}` —
la conformité d'une implémentation se mesure dessus (parsing exact,
champs, codes d'erreur), pas sur « ça a l'air de marcher ».
