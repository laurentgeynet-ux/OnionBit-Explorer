# Vol — protocole opérationnel

> **Étape 317** : la séquence de vol est mécanisée — deux
> commandes remplacent les checklists :
>
> ```powershell
> pwsh -File scripts/vol_start.ps1 -Mission <TAG> -Cibles <liste>
> #   alloc vol<N> atomique (claim = CLAIMED.json) → arbre propre
> #   → build release → spawn daemon + joblimit → imprime port/pid
> pwsh -File scripts/vol_close.ps1 -Vol <N>
> #   shutdown→kill pidfile → GC registre daemons → DONE → commit
> ```
>
> L'aide-mémoire de mission (endpoints, ordre des sondes,
> interdits) = `.devin/skills/vol/SKILL.md` — source unique.
> Ce fichier ne garde que les deux référentiels stables :
> les **signatures de mort** et les **règles dures**.

## Signatures de mort (cas par cas : vol-agent.md §7)

| Constat | Classe |
|---|---|
| sentinelle `panic:`/`rss_guard:`/`console:*`/`shutdown:api` | nommée — lire la sentinelle |
| `alive` + beats sains + `exit.log` présent | **kill sélectif** externe (`taskkill /F` → `EXIT:1`) |
| arbre entier mort + `exit.log` ABSENT | **job-close** — session lanceuse fermée (canal `session`) |
| `alive` + beat figé sur balise | stack overflow/spin — `drain`/`host` nomment le site |
| `alive` + `commit` au plafond | OOM/alloc-abort |
| beats vivants, `drain` fixe, `calls` figé | wedge — `session.ping` EAP nomme le verbe tenu |

## Règles dures

- **Verdict = binaire frais** — `Finished` vu + `exe.log`
  cité ; jamais sur WIP ni exe qui traîne (`vol_start`
  refuse l'arbre sale sauf `-Force`).
- **Arrêt = graceful d'abord** : `POST /api/shutdown` →
  `taskkill /PID /F` en dernier recours — jamais `/IM`
  (PID du pidfile `daemon-<port>.pid`, jamais à main levée).
- **Avant tout kill** : lire `Explorer-data\daemons\` —
  vivacité = pid + image + port lié sous ce pid (284). Le
  corpus est **partagé par jonction** entre clones : un seul
  registre physique, mais des daemons issus d'exes d'autres
  arbres — vérifier le profil annoncé, pas seulement le pid.
- **Job Object** : `vol_start` l'arme (il borne, ne tue pas —
  innocenté §B-135). `vol_memwatch` = chasses de fuite
  documentées seulement (sa signature = kill externe).
- **Un constat = mesure** — jamais « ça a l'air ».
- **Capture blanche ≠ verdict** — `/frame` de fin de dwell
  exigée (deux pulls byte-identiques espacés ~3-5 s =
  « stabilisé ») ; l'archive `<cible>.png` = la **dernière**
  frame, les précoces sont nommées `<cible>.t+<n>s.png`.
- **Vision = observateur actif** — chaque pull `/frame` arme
  la pompe de rendu (~4/s) ; noter « vision attachée t=… »
  pendant une fenêtre de mesure.

Post-mortem, gabarits, consentement, MCP/EAP, détail des
endpoints : `docs/vol-agent.md` (référence complète, ~900 l.).
Corpus : `Explorer-data/vols/README.md` (structure, git local).
