# Plan d'Implémentation 02 — Mode Radio & Écoute en Tâche de Fond (Audio-Only)

## 1. Contexte & Problématique

Des millions d'utilisateurs écoutent quotidiennement sur YouTube, Twitch ou des sites de replay des contenus essentiellement oraux :
- Podcasts, interviews, conférences, talk-shows, débats.
- Musique, sets DJ, radios en ligne.

### Le Gaspillage des Navigateurs Standards
Dans Chrome, Firefox ou Edge, dès qu'une vidéo YouTube est lue en arrière-plan (onglet minimisé ou masqué par une autre fenêtre) :
1. Le navigateur continue de télécharger un flux vidéo lourd (entre 3 et 15 Mbps en 1080p/4K).
2. Le CPU et le GPU décompressent 60 images par seconde en continu (calculs SIMD, transformées DCT, filtres de boucle, transferts mémoire).
3. **Conséquences** :
   - Autonomie de batterie des PC portables réduite de moitié.
   - Consommation de data forfaitaire inutile (plusieurs gigaoctets par heure).
   - Échauffement et bruit des ventilateurs.

---

## 2. Le Principe Architectural

Dans notre architecture MSE ([`ADR-0029`](file:///C:/OnionBit-Explorer/docs/architecture/decisions/0029-mse-surface-js-et-source-onionmedia.md)), le flux vidéo et le flux audio sont **totalement dissociés dès la source** via [`onionmedia_open_mse_multi`](file:///C:/Onion-explorer-media/crates/onionmedia-ffi/src/lib.rs) :

```
┌────────────────────────────────────────────────────────┐
│             Serveur Loopback MSE du Moteur             │
└──────────┬─────────────────────────────────┬───────────┘
           │ GET ?buffer=0 (Vidéo 1080p)     │ GET ?buffer=1 (Audio Opus)
           │ [10 à 15 Mbps]                  │ [128 kbps]
           ▼                                 ▼
   [ MseSource Vidéo ]               [ MseSource Audio ]
           │                                 │
           ▼                                 ▼
   [ Décodeur Vidéo MT ]             [ Décodeur Audio ]
   (VP9/AV1/H.264)                   (Opus/AAC)
           │                                 │
           ▼                                 ▼
   [ Rendu Texture D3D11 ]           [ Sortie Haut-parleurs ]
   
   ══════════════════════════════════════════════════════
   EN MODE "AUDIO-ONLY" :
   Le rail gauche (Vidéo) est TOTALEMENT SUSPENDU.
   Seul le rail droit (Audio) continue de fonctionner !
   ══════════════════════════════════════════════════════
```

---

## 3. Spécification Fonctionnelle

### 3.1. Déclenchement
1. **Mode Manuel (Bouton UI)** :
   - Un bouton toggle **"📻 Mode Radio (Audio Seul)"** sur la barre de contrôle vidéo et dans le menu d'onglet.
   - En un clic, l'affichage vidéo se fige sur une image de couverture fixe ou s'éteint, le flux audio continue sans aucune coupure de son.
2. **Mode Automatique Intelligent (Économie d'Énergie)** :
   - Option dans les préférences : *"Suspendre la vidéo si l'onglet est masqué plus de 15 secondes"*.
   - Dès que l'onglet est minimisé ou n'est plus actif, le navigateur coupe le rail vidéo.
   - Dès que l'utilisateur revient sur l'onglet, la vidéo reprend instantanément synchronisée sur le son en cours.

### 3.2. Bilan Énergétique et Réseau Estimé

| Paramètre | Mode Normal (1080p60) | Mode "Audio-Only" | Gain |
|---|---|---|---|
| **Bande passante réseau** | ~6 000 kbps | ~128 kbps | **-97.8 %** |
| **Data consommée (1h)** | ~2,7 Go | ~57 Mo | **Économie de 2,6 Go** |
| **Charge CPU / GPU** | 8 % à 25 % (selon CPU) | < 0.5 % | **-95 % d'effort processeur** |
| **Consommation batterie** | Élevée (décodage continu) | Négligeable (proche du repos) | **Autonomie multipliée par 2** |

---

## 4. Implémentation Technique

### 4.1. Côté Rust (`onionmedia` & `onionmedia-pipe`)
1. **Nouvelle méthode dans [`Player`](file:///C:/Onion-explorer-media/crates/onionmedia/src/player.rs)** :
   ```rust
   pub fn set_video_track_enabled(&mut self, enabled: bool) {
       // Si false :
       // 1. Met en pause la tâche MseSource vidéo (arrête les requêtes GET ?buffer=0).
       // 2. Met en sommeil le VideoThread (aucun décodage de frames).
       // 3. Libère la texture D3D11 d'affichage.
   }
   ```
2. **Gestion de la reprise (Re-synchronisation)** :
   - Quand `enabled` repasse à `true`, `MseSource` effectue un pull avec `?from={pts_audio_actuel}`.
   - Le décodeur vidéo décode la première keyframe et recolle instantanément sur l'horloge [`Clock`](file:///C:/Onion-explorer-media/crates/onionmedia-pipe/src/clock.rs) de l'audio.

3. **Symbole FFI dans `onionmedia-ffi`** :
   ```rust
   #[unsafe(no_mangle)]
   pub extern "C" fn onionmedia_set_video_enabled(
       player: *mut c_void,
       enabled: i32,
   ) -> i32
   ```

### 4.2. Côté Dart / Flutter (`OnionBit-Explorer`)
1. **Ajout dans [`OnionMediaPlayer`](file:///C:/OnionBit-Explorer/app/lib/features/browser/data/onionmedia_media_player.dart)** :
   ```dart
   void setVideoEnabled(bool enabled) {
     _setVideoEnabled?.call(_h, enabled ? 1 : 0);
   }
   ```
2. **Surveillance de visibilité dans [`browser_controller.dart`](file:///C:/OnionBit-Explorer/app/lib/features/browser/presentation/browser_controller.dart)** :
   - Écoute des changements d'onglet actif (`activeTabIdProvider`).
   - Timer de 15 secondes d'inactivité avant suspension automatique du rail vidéo.
   - Réveil immédiat lors du re-focus de l'onglet.

---

## 5. Feuille de Route

- [ ] **Étape 1** : Implémenter `set_video_track_enabled` dans `onionmedia-pipe` et l'export FFI.
- [ ] **Étape 2** : Ajouter le switch "Audio-Only" dans la barre de contrôle média Flutter.
- [ ] **Étape 3** : Valider sur un stream YouTube réel (vérification de la baisse de débit dans le moniteur réseau).
- [ ] **Étape 4** : Implémenter l'automatisation en arrière-plan avec réveil sans glitch audio.
