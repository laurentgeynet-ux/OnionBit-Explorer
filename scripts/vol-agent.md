# Vol agent — capture de crash daemon (WER LocalDumps)

Contexte : mort silencieuse intermittente du daemon en vol
(journal x.com — processus absent, port libéré, zéro trace
panic, non reproduit en ~55 inputs de campagne). Signature
probable = stack overflow (pas d'unwind sous Windows) ou kill
externe.

## Capture — WER LocalDumps (armé)

La capture est **externe OS** : tout crash futur du daemon
écrit un minidump/full dump complet, même sur les fautes où
un handler in-process ne tourne plus (pile épuisée).

Clé armée sous **HKCU** (zéro admin) :

```
HKCU\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\explorer-daemon.exe
  DumpFolder = C:\CrashDumps
  DumpType   = 2  (full dump)
```

Vérifier un dump après un vol :

```powershell
Get-ChildItem C:\CrashDumps -Filter *.dmp | Sort LastWriteTime -Desc
```

Le `.dmp` s'ouvre dans WinDbg / Visual Studio — pile native
au point de faute, mécanisme de mort nommé définitivement.

## Durcissement piles (Étape 138 — livré)

- Runtime tokio : `thread_stack_size(32 Mio)` — workers ET
  pool `spawn_blocking` (`explorer-daemon/src/main.rs`).
- Thread moteur jscore : `.stack_size(32 Mio)`
  (`explorer-js`, JS-C41 — parseur/évaluateur descendant
  mesuré ~8–55 Kio/niveau).
- SEH/minidump in-process : **écarté** — `forbid(unsafe)`
  + le handler court sur la pile épuisée ; WER le remplace
  strictement.

## Watchdog processus (déjà livré)

`scripts/spawn_daemon.ps1` — spawn `SW_HIDE` + watchdog qui
écrit `EXIT:!ERRORLEVEL!` dans `daemon-<port>.exit.log` :
kill externe → `-1`, crash → code exception, absence = kill
d'arbre (fermeture console parente).

`scripts/daemon_watch.ps1` — cycles répétés + CSV RSS
(fuite mémoire suspecte `RADAR_PRE_LEAK_64` : RSS stable
640 Mo mesuré campagne x.com — exonéré).
