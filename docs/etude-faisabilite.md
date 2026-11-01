# Étude de faisabilité : client YouTube en Rust + GPUI

*1er novembre 2026. Tests faits en direct contre YouTube (client WEB `2.20261029.11.00`).*

## Verdict

| Fonctionnalité | Verdict | Source des données | Connexion requise |
|---|---|---|---|
| Page Home (recommandations) | 🟡 faisable, avec des risques | InnerTube `browse` `FEwhat_to_watch` + cookies | **Oui** |
| Liste des abonnements | 🟢 | InnerTube `browse` `FEchannels` (non testé), ou import Google Takeout | Oui (sauf Takeout) |
| Dernières vidéos des abonnements | 🟢 | InnerTube `FEsubscriptions`, ou RSS par chaîne en secours | Non avec le RSS |
| Commentaires | 🟢 | InnerTube `next` (continuation) | Non |
| Like/dislike (RYD) | 🟢 | `returnyoutubedislikeapi.com/votes` | Non |
| Lecteur vidéo | 🟢 rendu (repris de peeby), démarrage ~1 s / 🟡 client non officiel | libmpv + surface D3D11 de peeby, HLS via InnerTube `VISIONOS`, yt-dlp en repli | Non (sauf contenu restreint) |

Le projet est faisable. Les deux zones de risque sont la **Home** (elle exige une session Google valide) et l'**extraction des flux vidéo** (YouTube la durcit régulièrement ; le rendu, lui, est réglé par peeby). Il ne faut rien écrire soi-même côté flux vidéo : on délègue à yt-dlp, qui est maintenu activement.

---

## 1. Page Home : le point dur

### Ce que j'ai testé

Un `POST /youtubei/v1/browse` avec `browseId: "FEwhat_to_watch"` **sans connexion** renvoie HTTP 200 avec :

- `serviceTrackingParams` → `logged_in = 0` ;
- un `richGridRenderer` **vide** ;
- un `feedNudgeRenderer` : *« Votre historique YouTube est désactivé »*.

Depuis 2024, YouTube ne propose plus de recommandations anonymes : **sans session, la Home est vide**. Pour la même raison, elle reste vide pour un compte connecté dont l'historique des vidéos regardées est en pause.

### Comment obtenir la Home personnalisée

Il n'existe pas d'API officielle : la Data API v3 n'a plus d'endpoint de recommandations depuis 2016. La seule voie est l'API privée **InnerTube** du site web, appelée avec les cookies d'une session Google :

- en-têtes `Cookie` (cookies `.youtube.com`), `Origin`/`X-Origin: https://www.youtube.com`, `X-Goog-AuthUser: 0`, `X-Youtube-Client-Name: 1`, `X-Youtube-Client-Version` ;
- `Authorization: SAPISIDHASH <ts>_<sha1> SAPISID1PHASH … SAPISID3PHASH …`, où `sha1 = SHA1("<ts> <SAPISID> https://www.youtube.com")` ;
- corps minimal : `{"context":{"client":{"clientName":"WEB","clientVersion":…}},"browseId":"FEwhat_to_watch"}`.

D'autres projets confirment que cela marche aujourd'hui. Par exemple, piTube (PR récente) obtient `logged_in=1` et 24 vidéos avec exactement cette recette.

**Le OAuth « TV » ne marche plus.** YouTube l'a bloqué fin 2024 et yt-dlp l'a retiré. **Les cookies sont la seule méthode d'authentification viable.**

### Format de réponse (vérifié)

Home, abonnements et onglet Vidéos d'une chaîne utilisent tous la même grille :

```
twoColumnBrowseResultsRenderer > tabs[0].tabRenderer.content
  > richGridRenderer.contents[]
      > richItemRenderer.content.lockupViewModel      ← vidéo (contentType = LOCKUP_CONTENT_TYPE_VIDEO)
      > richSectionRenderer > … shortsLockupViewModel ← étagère de Shorts
      > continuationItemRenderer                      ← page suivante
```

Il n'y a plus de `videoRenderer` : tout passe par `lockupViewModel` (`contentId`, `metadata.lockupMetadataViewModel.title.content`, `metadataRows[].metadataParts[].text.content`, miniatures dans `contentImage.thumbnailViewModel`, durée dans le badge). **Un seul parser couvre donc la Home, les abonnements et les chaînes.** Il faut l'écrire de façon tolérante (`serde_json::Value` et accès défensifs plutôt que des structs strictes) : le décodeur typé de piTube perdait 24 items sur 24 au dernier changement de format.

### Obtenir les cookies (UX de connexion)

1. **v1 : import d'un `cookies.txt`.** L'utilisateur se connecte dans une fenêtre de navigation privée, exporte les cookies, puis **ferme la fenêtre** : un onglet ouvert fait tourner (« rotate ») les cookies et invalide l'export. C'est la méthode recommandée par yt-dlp.
2. **Plus tard : connexion via une WebView intégrée** (WebView2 / `wry`), comme le fait Grayjay, en lisant les cookies via le CookieManager. Google bloque parfois la connexion dans une vue intégrée (« ce navigateur n'est peut-être pas sécurisé »), c'est donc à valider.
3. **Lire directement les cookies du navigateur** (crate `rookie`) : Firefox fonctionne. Chrome sur Windows utilise le chiffrement *app-bound* depuis la v127, ce qui rend cette lecture peu fiable.

Une fois connecté, le client doit **persister les `Set-Cookie`** renvoyés par YouTube (notamment `__Secure-1PSIDTS`), sinon la session expire.

### Risques Home

- L'API est privée et non documentée : le format change sans préavis. Il faut prévoir de la maintenance.
- Les CGU de YouTube interdisent les clients tiers. Le risque pour le compte est faible en pratique pour un usage personnel, mais il n'est pas nul.
- Un échec silencieux est le piège principal : HTTP 200 avec `logged_in=0` renvoie une Home vide sans erreur. **Il faut toujours vérifier `logged_in`.** Le POC le fait.

---

## 2. Abonnements et dernières vidéos

- **Liste des chaînes** : `browse` `FEchannels` (connecté ; *non testé*, RustyPipe utilise de son côté un token de continuation codé en dur). Alternative sans connexion : importer le `subscriptions.csv` de Google Takeout.
- **Flux des abonnements** : `browse` `FEsubscriptions` (connecté), avec **le même parser que la Home**.
- **Secours sans connexion** : `https://www.youtube.com/feeds/videos.xml?channel_id=UC…`. Testé : HTTP 200, 15 dernières vidéos par chaîne. C'est stable depuis des années ; le client fusionne et trie lui-même. C'est l'approche de FreeTube et NewPipe. Pas de Shorts/live distingués et 15 items max, mais c'est très robuste.

## 3. Page vidéo : commentaires et likes

- **Métadonnées + vidéos suggérées** : `POST /youtubei/v1/next` avec `videoId`. Les suggestions sont aussi des `lockupViewModel`, donc le même parser s'applique.
- **Commentaires** (testés **sans connexion**) : prendre le token `continuationCommand` de l'`itemSectionRenderer`, puis rappeler `/next` avec `continuation`. Le contenu **n'est plus dans les renderers** mais dans `frameworkUpdates.entityBatchUpdate.mutations[].payload.commentEntityPayload` (auteur, texte `properties.content.content`, date, `toolbar.likeCountNotliked`, `toolbar.replyCount`). On a obtenu 20 commentaires par page, avec les tokens de tri (« populaires » / « récents ») dans l'en-tête.
- **Dislikes** : `GET https://returnyoutubedislikeapi.com/votes?videoId=…` renvoie `likes`, `dislikes`, `rating`, `viewCount` (testé). Conditions : **100 req/min et 10 000 req/jour par client, avec attribution obligatoire** (lien vers returnyoutubedislike.com). Il faut mettre un cache local.

## 4. Lecteur vidéo

### Récupérer les flux : ne pas le faire soi-même

- YouTube impose désormais des **PO tokens** (BotGuard), du **SABR** sur le client web et un **déchiffrement de signature en JS**. yt-dlp exige un runtime JS externe (Deno) depuis novembre 2025.
- **RustyPipe est à écarter** : dernière release en avril 2025, pas de méthode Home, et un test d'octobre 2026 montre que le déchiffrement de signature est cassé (« could not extract sig fn name »).
- **Solution retenue : yt-dlp + Deno**, invoqués par **mpv** via son hook `ytdl` intégré (`mpv https://youtu.be/ID`). Les cookies de la session sont passés à yt-dlp (`cookies=…`). yt-dlp et Deno sont **embarqués avec l'app** (voir les résultats plus bas). Pour suivre les changements de YouTube, il suffit de mettre à jour yt-dlp, sans recompiler l'app.

### Afficher la vidéo dans GPUI : reprendre la chaîne de peeby

Le projet `D:\DEV\peeby` (client Jellyfin, GPUI + libmpv) a déjà résolu ce problème, et mieux que la fenêtre enfant `--wid` que j'envisageais au départ :

- **GPUI `gpui-pre =0.3.10`** + **gpui-component 0.8.1** (ADR 0006), versions épinglées ensemble.
- **libmpv chargée au runtime** avec `libloading`, FFI écrite à la main dans `player::sys` (ADR 0007). Build shinchiro `20261021-git-e471496c564`.
- **Surface GPU sans copie** (ADR 0008) : mpv rend avec `MPV_RENDER_API_TYPE_D3D11` dans un ring de 3 textures partagées (keyed mutex). Un patch d'environ 100 lignes de `gpui-pre-windows` (`vendor/`) permet à GPUI de les dessiner directement. Mesuré à environ 0,2 cœur en 1080p60, 8 à 9 fois moins que le rendu logiciel, qui reste disponible en secours (`SwSurface`).
- **Les contrôles sont de vrais éléments GPUI par-dessus la vidéo**, ce que la fenêtre enfant ne permettait pas.

**Compatibilité YouTube vérifiée sur le binaire** : `third_party/mpv/libmpv-2.dll` embarque **LuaJIT et le script `ytdl_hook`**. La crate `player` démarre la lecture par un `loadfile <url>` générique : avec une URL `https://www.youtube.com/watch?v=…`, mpv appelle yt-dlp lui-même, sans rien changer à la chaîne de rendu. Il faut seulement ajouter à `mpv_options` (crates/player/src/lib.rs:458) :

| Option mpv | Rôle |
|---|---|
| `ytdl=yes` | active le hook (défaut mpv, à forcer explicitement) |
| `script-opts=ytdl_hook-ytdl_path=<…>\yt-dlp.exe` | yt-dlp fourni avec l'app, mis à jour par `yt-dlp -U` |
| `ytdl-format=bv*[height<=?1080]+ba/b` | vidéo et audio séparés, qu'yt-dlp fusionne dans une EDL lue par mpv |
| `ytdl-raw-options-append=cookies=<cookies.txt>` | vidéos à restriction d'âge, membres, Premium |

C'est fait dans la copie (`crates/player`) : `StartOptions::ytdl` (chemin de yt-dlp, format, options transmises à yt-dlp via `change-list ytdl-raw-options append`) et `StartOptions::log_file`.

### Résultats du spike `spikes/yt-play` (1er nov. 2026, RX 9070 XT)

**Embarquer yt-dlp sans rien installer sur l'hôte** : `scripts/fetch-tools.ps1` télécharge des versions épinglées, vérifiées par SHA-256, dans `third_party/yt-dlp/`. Le bundle de l'app livrera la même arborescence dans `tools/` à côté de l'exécutable.
- **yt-dlp 2026.10.26**, build `yt-dlp_win.zip` (« onedir ») : rien n'est décompressé dans `%TEMP%` à chaque appel, et `--version` répond en 0,4 s.
- **Deno 2.9.8**, environ 110 Mo, passé explicitement par `--js-runtimes deno:<chemin>`.
- **Isolation** : `--ignore-config` (on ignore la config yt-dlp de l'hôte) et `--cache-dir` pointé vers le dossier de l'app. Sans ça, yt-dlp écrit dans `~/.cache/yt-dlp`.
- **QuickJS-NG (2 Mo) écarté** : 5,6 s par challenge JS, contre 0,8 s pour Deno.

| Mesure | Résultat |
|---|---|
| Rendu | GPU sans copie (`VO: [libmpv] 1920x1080 d3d11[nv12]`) |
| Décodage | **matériel** `d3d11va`, AV1 (format 399) ou VP9 selon la vidéo, audio Opus |
| CPU pendant la lecture 1080p | environ 1,5 s de CPU pour 25 s de process, yt-dlp compris |
| yt-dlp seul (Deno) | 2,7 à 3,4 s : page, API player, JS du player, challenge JS (0,8 s) |
| Démarrage complet, vidéo de 19 s | **3,0 s** |
| Démarrage complet, clips de 3 à 4 min | **8,0 à 8,8 s** |

**Où passent les secondes** :
1. **Sous-titres** : par défaut, `ytdl_hook` demande toutes les langues, et mpv ouvre les pistes `timedtext` avant de lancer la lecture (+5 s mesurées). Il faut passer `sub-langs=none` au démarrage, puis ajouter la piste choisie avec `sub-add` quand l'utilisateur en sélectionne une.
2. **`available_at`, imposé par YouTube** : les URLs de flux sont délivrées avec une date de validité environ 5 s dans le futur. J'ai vérifié qu'avant cette date le serveur répond **403**, et **206** juste après. `ytdl_hook` attend donc (« Waiting 5.00 seconds for the stream to become available »). Les clients sans attente (`tv_simply`, `android_vr`) ne donnent que du 360p (format 18). **Pour du 1080p, l'attente est incontournable.**

### Démarrage en ~1 s : le client InnerTube `VISIONOS` (HLS)

En analysant les formats renvoyés par yt-dlp, on voit que **les formats HLS (`m3u8_native`) n'ont pas d'attente `available_at`** : leur échéance est déjà passée quand yt-dlp répond, avec une marge de -1,6 à -1,9 s sur 4 vidéos. Seuls les formats `https` (DASH) sont retardés. Forcer le HLS dans yt-dlp fait déjà passer le démarrage de 8,5 s à 4 s.

Ces HLS viennent du client **`visionos`** de yt-dlp (`REQUIRE_JS_PLAYER: False`). Testé client par client, c'est le seul qui donne du 1080p sans challenge JS ni attente : `tv_simply` et `android_vr` plafonnent à 360p, `web_safari` impose l'attente. L'URL du manifeste est **déjà signée par le serveur**, sans paramètre `n` à résoudre. L'app peut donc se passer de yt-dlp :

1. **Au démarrage de l'app**, un `browse` anonyme fournit un `visitorData` (0,2 s, une fois par session). Sans lui, la plupart des vidéos répondent `LOGIN_REQUIRED`.
2. **Au clic**, un `POST /youtubei/v1/player` avec le contexte VISIONOS de yt-dlp 2026.10.26 (`clientVersion 1.02`, `X-Youtube-Client-Name: 101`, UA Safari) et le visitorData, puis le GET du manifeste maître. Ensemble : **0,16 à 0,37 s**.
3. Dans le manifeste, on choisit le variant vidéo (au plus 1080p, VP9 de préférence) et sa piste audio : les variants de YouTube sont **vidéo seule**, et l'audio est un `EXT-X-MEDIA` désigné par l'attribut `AUDIO=` (groupe 234, AAC 128k). On donne à mpv l'URL du variant et `audio-files` (ajouté à `player` : `StartOptions::external_audio`), avec le `User-Agent` Safari.

Mesures (spike `PEEBTUBE_RESOLVER=visionos`, le mode par défaut, 5 vidéos) :

| Étape | Temps |
|---|---|
| Résolution (player + manifeste) | 0,16 à 0,37 s |
| mpv : manifestes et premiers segments jusqu'à « Starting playback » | environ 0,66 s |
| **Clic → lecture**, son compris | **environ 1,0 à 1,5 s** (indicateur 1,3 à 2,0 s, qui surestime d'environ 0,25 s) |
| Démarrage à 150 s (équivalent d'un seek) | 0,95 s |

Tout en **1080p VP9, décodage matériel `d3d11va`**, rendu GPU sans copie, audio AAC via WASAPI. Le manifeste propose aussi du 1440p et du 2160p en VP9, mais pas d'AV1.

**Pour descendre sous 1 s** :
- Résoudre en avance, dès l'ouverture de la page vidéo ou au survol d'une miniature : les URLs restent valides 6 h (`expire`, vérifié). Cela retire 0,2 à 0,4 s.
- À l'ouverture de la page, ouvrir la vidéo dans mpv **en pause** : le clic sur lecture devient alors instantané.

**Risques et repli** :
- VISIONOS est un client non documenté, que YouTube peut bloquer du jour au lendemain. yt-dlp l'utilise aujourd'hui, et son source sert de référence pour suivre les changements (version du client, en-têtes).
- **Garder yt-dlp en repli** : si le client renvoie un statut autre que `OK` (âge, membres, compte requis, client bloqué) ou pas de `hlsManifestUrl`, l'app bascule sur yt-dlp + Deno, avec les cookies si connecté. Il faut alors forcer le HLS pour éviter l'attente : `bv*[protocol=m3u8_native][height<=?1080]+ba[protocol=m3u8_native]/…`. Compter 3 à 4 s dans ce cas.
- Pas encore testés avec ce client : vidéos à restriction d'âge, membres, lives, Shorts.

**Pas encore testé** : seek pendant la lecture (le démarrage à 150 s, lui, fonctionne), lecture longue, vidéos à restriction d'âge avec cookies, lives.

## 5. Architecture proposée

Partir de la structure de peeby et en réutiliser les crates (même auteur, même licence GPL) :

```
peebtube/
  crates/innertube/   remplace jellyfin-api : reqwest + serde_json, auth cookies/SAPISIDHASH,
                      parser lockupViewModel/commentEntityPayload, persistance Set-Cookie
  crates/ryd/         client Return YouTube Dislike + cache
  crates/player/      repris de peeby (+ options ytdl)
  crates/gpui-video/  repris de peeby tel quel
  vendor/gpui-pre-windows/  patch D3D11 de peeby
  crates/peebtube/    app GPUI : reprendre de peeby runtime.rs (pont tokio ↔ GPUI),
                      images.rs (ImageStore LRU + cache disque, ici pour i.ytimg.com),
                      theme/motion/ui, player_page.rs (contrôles)
```

Il vaut mieux sortir `player`, `gpui-video` et le patch vendor dans un dépôt partagé (ou un workspace git commun) plutôt que les copier : sinon chaque correction du patch GPUI doit être reportée deux fois.

## 6. POC Home : `poc-home/`

```
cargo run --release -- --anon                 # testé : HTTP 200, logged_in=0, Home vide + message YouTube
cargo run --release -- --parse browse.json    # testé sur l'onglet Vidéos d'une chaîne : 30/30 vidéos parsées
cargo run --release -- cookies.txt            # NON testé (je n'ai pas de session) : à lancer de ton côté
```

Le mode cookies échoue explicitement si `logged_in != 1`. Dans ce cas, copie la requête `/youtubei/v1/browse` de la Home depuis les devtools (« Copy as cURL ») et compare ses en-têtes avec ceux du POC : le format exact du hash `SAPISID*HASH` est la partie la plus susceptible d'avoir changé.

## Sources

- [piTube PR #12 : Home connectée via cookies + SAPISIDHASH](https://github.com/omersusin/piTube/pull/12)
- [tombulled/innertube : browse `FEwhat_to_watch`](https://github.com/tombulled/innertube)
- [yt-dlp : OAuth retiré, cookies seulement](https://github.com/yt-dlp/yt-dlp/wiki/Extractors)
- [yt-dlp PO Token Guide](https://github.com/yt-dlp/yt-dlp/wiki/PO-Token-Guide), [web = SABR uniquement (#12482)](https://github.com/yt-dlp/yt-dlp/issues/12482), [runtime JS externe (#15012)](https://github.com/yt-dlp/yt-dlp/issues/15012)
- [Test RustyPipe d'octobre 2026 (den-hearth PR #6)](https://github.com/oxyc/den-hearth/pull/6), [RustyPipe `userdata.rs`](https://docs.rs/crate/rustypipe/latest/source/src/client/userdata.rs)
- [Return YouTube Dislike : API](https://returnyoutubedislike.com/docs/fetching), [conditions d'utilisation](https://returnyoutubedislike.com/docs/usage-rights)
- [GPUI : `HasWindowHandle` sur `Window` (#24327)](https://git.secluded.site/zed/commit/4270f89956b573512984c6bcbfb9544879381461), [raw-window-handle X11 / viewport enfant (#50768)](https://git.secluded.site/zed/commit/608185be4e0cc32ad925074185b5821af8aa2da9)
- [gpui-video-player (GStreamer)](https://lib.rs/crates/gpui-video-player), [alhazen-gpui](https://docs.rs/crate/alhazen-gpui/latest)
