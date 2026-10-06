> Rapport historique M9, conservé comme baseline. État actuel : [STATUS.md](STATUS.md).

# WOOMY / MANMENMI M9 — CAFE RUNTIME FOUNDATION

Jalon vérifié le **2 octobre 2026**, Windows x64, Rust 1.98.1, MSVC 19.44.35229. Woomy consomme le **MANMENMI M9 existant** fourni par l'utilisateur. Le préfixe d'entrée franchit ses deux premiers imports dans un modèle explicite et borné. **Le jeu ne démarre pas ; aucune équivalence de contexte Cafe réel n'est revendiquée.**

## Baseline et vérifications actuelles

| Mesure | Avant ce jalon | Résultat actuel |
| --- | ---: | ---: |
| Tests workspace | 111 réussis, 0 échec, 1 ignoré | **125 réussis, 0 échec, 2 ignorés** |
| Test externe M9, normalement ignoré | absent | **1 réussi**, exécuté explicitement avec le helper compilé |
| Validateur PPC/IR | 512 MATCH | **512 MATCH** |
| Validateur natif scalaire | 5 120 MATCH | **5 120 MATCH** |
| Validateur multi-fonctions | 6 144 MATCH | **6 144 MATCH** |
| Fonctions réelles, domaine synthétique | une campagne 512 revérifiée en baseline ; quatre présentes | **4 × 512 = 2 048 MATCH** PPC/IR/natif |
| Contextes réels validés / handlers de référence | 0 / 0 | **0 / 0** |
| Original dump | 615 fichiers intacts | **615 fichiers intacts**, 1 761 602 316 octets |

Les sept contrôles finaux sont tous réussis : `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`, `cargo build --release` et les trois validateurs historiques. Journaux : `target/manmenmi-baseline.{json,log}` et `target/manmenmi-final.{json,log}`. Les validations sont sérialisées pour éviter les verrous d'exécutables Windows.

Les 14 nouveaux tests exécutés par défaut comprennent 10 tests de descripteurs/matérialisation, 3 tests d'intégration image/barrières/identité/replay et un doctest qui interdit la conversion d'adresse guest en pointeur hôte. Le test externe supplémentaire crée deux vrais processus M9 isolés ; il couvre les deux états debugger, absence de thread/état, mappings, identité/versions et callbacks mémoire avec bounds/permissions/overlap/écritures ordonnées. Il est lancé avec `WOOMY_MANMENMI_ADAPTER=<helper.exe> cargo test -p woomy-manmenmi --test external -- --ignored`. Aucun octet Nintendo dans les fixtures du dépôt. Aucune campagne fuzz ni suite complète GPU/audio de M9 n'est revendiquée.

## Versions et architecture

| Contrat | Version actuelle |
| --- | --- |
| SQLite | **6**, aucune migration ajoutée/modifiée pour ce jalon |
| Parser / PPC / CFG / dataflow | 1 / 6 / 4 / 5 |
| IR / C++ backend / Native ABI | **3 / 3 / 2** |
| Woomy runtime ABI | **2** (précédemment 1) |
| GuestMemory / calls / summaries | 1 / 1 / 2 |
| Image / loader / relocation / context | 1 / 1 / 1 / 1 |
| Process descriptor / thread model / provider ABI / import semantics | **1 / 1 / 1 / 1** |
| M9 runtime / thread / service ABI ; bridge | **1 / 1 / 1 ; 1** |
| Snapshot / replay | **1 / 1** |

Woomy possède CPU, image, IR, dispatch et mémoire guest. M9 C++ possède CafeRuntime/CafeProcess/CafeThread et les services. La crate `woomy-manmenmi` fournit les descripteurs d'entrée et un transport borné vers un helper C++20 lié à huit unités core/cafe existantes. Elle ne duplique pas les services en Rust. Aucun changement aux sources M9 ; 22 empreintes de sources/en-têtes consommés vérifiées dans le manifeste local `.woomy/manmenmi.json`.

Programme, Machine et provider vérifient une identité commune incluant versions, module/image, domaine, état process/thread et empreinte du helper. Les **18 artefacts natifs** ont été reconstruits parce que runtime ABI 2 les invalide ; les anciens résultats restent historiques et les quatre campagnes synthétiques ont été rejouées avec la nouvelle version. Le transport natif n'accepte pas encore de contexte M9 : il refuse explicitement de perdre cette identité.

Architecture, commandes, invariants et procédure de capture : [PROCESS.md](PROCESS.md). Contrats : [CALLS.md](CALLS.md), [CONTEXT.md](CONTEXT.md), [NATIVE.md](NATIVE.md). Sources et classes d'évidence : [SOURCES.md](SOURCES.md).

## Processus fourni à l'entrée et services

L'entrée locale est `0x02C6BCA4`, module `module.Gambit.30647d865809`, hash module `45ecd1ed81a97f46b1c8ae24820cd47fbc55399b50af09bf1f9050fe803308e5`.

Le profil **CAFE_BOOTSTRAP_MODEL** fournit un thread guest `0x70000000` (stockage `0x6A0`), une pile `[0x70010000,0x70020000)`, r1=`0x7001FF00`, core 1 et un debugger explicitement absent ou présent. Les autres registres sont explicitement initialisés à zéro, sauf LR sentinelle `0xFFFF0003`. Il s'agit de choix de modèle, jamais d'adresses ou de valeurs capturées du jeu. Sans profil, les valeurs restent inconnues.

Les champs de thread établis sont écrits en big endian ; zéro-fill est une initialisation explicite du modèle. Les chunks capturés conservent leurs octets et leurs trous non mappés. La disposition OSThread/OSContext et les déclarations d'API sont des **PUBLIC_IMPLEMENTATION_OBSERVATION** WUT, pas une spécification officielle. Le retour scalaire/pointeur en r3 est corroboré par l'ABI PPC (**DOCUMENTED**) et les usages du dump (**EMPIRICALLY_OBSERVED_IN_LOCAL_DUMP**). Adresses et initialisation sont **WOOMY_INFERRED**, vérifiées sur fixtures **SYNTHETIC_VALIDATED**. L'état Cafe réel reste **UNKNOWN**, sans preuve **REFERENCE_VALIDATED**.

M9 expose six services de modèle ; Woomy en active deux pour l'entrée :

| Service effectivement appelé dans M9 | Contrat de modèle | Référence |
| --- | --- | --- |
| `coreinit::OSGetCurrentThread` | zéro argument ; r3 reçoit l'adresse guest associée au thread courant, ici `0x70000000` ; état manquant = erreur | non validée |
| `coreinit::OSIsDebuggerInitialized` | zéro argument ; r3 reçoit false/true explicite ; état inconnu = erreur | non validée |

Les deux contrats modifient seulement r3 et ne modifient pas la RAM guest ; LR est géré par le dispatcher. La préservation des autres registres est limitée au modèle. ABI Cafe complète, clobbers, effets internes, timing et synchronisation restent inconnus. Les résumés statiques conservent UNKNOWN. Les quatre autres services M9 ne sont pas activés dans le provider du jeu, car aucun n'est le prochain obstacle observé.

## Entry frontier avant/après

| Contexte | Instructions | Blocs terminés | Appels réussis | Prochain obstacle |
| --- | ---: | ---: | ---: | --- |
| Avant provider, contexte mémoire supposé (analyse statique) | **8** | **0** | **0** | OSGetCurrentThread, site `0x02C6BCC4` |
| Modèle debugger absent, réellement exécuté PPC/IR | **13** | **3** | **2** | avant instruction `0x02C6BCF0` : `$UNDEF` au champ `0x02C6BCF2` |
| Modèle debugger présent, réellement exécuté PPC/IR | **19**, trap inclus | **4** | **2** | trap à `0x02C6BCEC` ; contrôle Cafe/debugger inconnu |

Chaque parcours produit cinq écritures ordonnées sur la pile. État complet, mémoire, trace et événements d'appels PPC/IR concordent ; les deux résultats restent **INCONCLUSIVE**, pas MATCH. Aucune exécution native de ce préfixe n'est annoncée. `entry frontier` énumère les deux possibilités sans choisir un chemin dynamique ; `entry run-model` exige un profil explicite.

**Blocage exact sans debugger :** record relocation **280547**, section de relocation 7, cible section 2, type **6 (ADDR16_HA)**, table de symboles 35/index 3, symbole `$UNDEF`, addend 0, place **`0x02C6BCF2`**. L'absence de liaison légitime interdit de donner une valeur à l'instruction à `0x02C6BCF0`. Ses octets initiaux zéro ne sont pas une résolution. L'instruction ultérieure `addi r3,r29,0x408` ne prouve pas la sémantique d'un champ de thread et n'est pas atteinte sur ce parcours.

## Image, couverture, contextes et fermeture

L'image reste `77fe959643e3db31964dd743fce074884cdb373b0216edd9b9063076cbb5d273` : **6 régions**, 16 069 688 octets, 531 564 octets NOBITS zéro, 2 RX et 4 RW. Sur **402 508 relocations**, 393 068 appliquées avec preuve locale, 9 423 imports différés, 17 `$UNDEF` non supportées, 0 ambiguë/invalide. Cette image à adresses liées n'est pas un processus vivant.

**115 fonctions, 5 187 instructions, 1 283 blocs, 1 658 edges** ; **18 LIFTED, 97 PARTIALLY_LIFTED, 0 INVALID**. Les quatre instructions UNKNOWN restent inchangées. Histogramme primaire : 459 effets d'appel inconnus, 27 relocations, 21 XER, 8 CR, 4 branches conditionnelles, 4 entiers, 2 mémoire, 4 instructions inconnues et 1 contrôle indirect.

Les **18 contextes** sont revérifiés : **4 REGISTER_STATE_ONLY**, **14 CONTROLLED_MEMORY_REQUIRED**, tous **CONTEXT_INCOMPLETE** pour une référence réelle. Le nouveau modèle d'entrée ne les promeut pas rétroactivement. Les quatre campagnes synthétiques restent distinctes :

| Suffixe de fonction Gambit | Cas | PPC/IR | PPC/natif | IR/natif | Mismatch / inconclusive |
| --- | ---: | ---: | ---: | ---: | ---: |
| `027967CC` | 512 | 512 | 512 | 512 | 0 / 0 |
| `02798514` | 512 | 512 | 512 | 512 | 0 / 0 |
| `02838B10` | 512 | 512 | 512 | 512 | 0 / 0 |
| `02852BB4` | 512 | 512 | 512 | 512 | 0 / 0 |

Fermeture d'entrée : **115 fonctions connues, 18 éligibles, 97 partielles, 51 obstacles de contrôle**, non fermée. **459 sites d'appel** : 353 internes, 70 imports, 36 indirects ; catalogue **35 symboles / 3 bibliothèques**. Les deux contrats de modèle n'annulent pas les obstacles de référence. Les **41 indirects historiques hors retours** restent **0 résolu / 41 non résolus** ; les 167 lignes de sites, retours inclus, sont identiques à la baseline.

## Snapshots et CLI

Le snapshot local existant garde l'origine **SYNTHETIC**. `snapshot inspect`, `validate` et `replay` passent ; replay donne **REPLAY_EXECUTION_AGREEMENT** PPC/IR/natif, sans REAL_GAME_CONTEXT_MATCH. Les fichiers sont bornés et vérifiés contre image, fonction, entrée exacte, permissions et hashes. Aucun replay arbitraire à mi-bloc. Le sidecar process M9, une capture externe légitime et son attestation restent manquants pour les références exigeant ces services ; la procédure et les données nécessaires sont documentées dans PROCESS.md.

**58 commandes finales** ont retourné 0 : inspection process/thread inconnus et modèles, frontière statique, deux exécutions M9, trois commandes snapshot, 18 builds, quatre validations synthétiques, 18 contextes, runtime/imports/catalogue/fermeture/couverture/indirects/resume/status. Quatre contrôles négatifs refusent profil absent/invalide et traversée snapshot. Le shell avec entrées CRLF est vérifié. `resume` réutilise **1 module**, sans reparsage. Le build de connexion M9 avait déjà réussi ; son handshake et les deux handlers sont vérifiés à chaque exécution.

## Régressions, intégrité et exécutables

Aucun mismatch sémantique dans les cas exécutés. Le premier build du helper avait rencontré MSVC C1083 sur des chemins Windows canoniques `\\?\` : l'adaptateur de compilation normalise ces chemins. Les contrôles Clippy et les tests ont corrigé les erreurs de développement sans réduire les assertions. Une corruption d'accents lors de génération de documentation a été corrigée. L'invalidation runtime ABI 2 est intentionnelle et les 18 artefacts sont reconstruits.

Inventaire avant/après : **615 fichiers, zéro modifié/manquant/inattendu**, 1 761 602 316 octets. SQLite `integrity_check = ok`, aucune violation de clé étrangère. Les 22 sources/en-têtes M9 consommés ont conservé leur SHA-256. Aucun fichier de `code/`, `content/`, `meta/` ni de M9 n'a été modifié. Les dérivés, binaires de jeu, snapshots et rapports restent sous `.woomy` ignoré ; aucun commit n'a été créé.

Les deux copies `target/release/woomy.exe` et `Splatoon/Games/Splatoon/woomy.exe` sont identiques : **10 406 912 octets**, SHA-256 **`26116890d267abe83c14b91671e098228111e966884d34234fb3c4cdb65787ad`**.

Le helper M9 compilé, distinct de Woomy, a pour SHA-256 `e3b8b31f4d497a05d8e29821be05eb38335af4895bcf7ad6459daa04e805e1ef`. Rapports locaux : `.woomy/reports/manmenmi-final-cli.json`, `entry-model-debugger-{absent,present}.json`, `manmenmi-integrity-after.json`, `manmenmi-final-integrity-checks.json`, `manmenmi-executables.json` et `manmenmi-final-summary.json`.

Le prochain travail justifié est d'établir la **liaison réelle de `$UNDEF`** à `0x02C6BCF2`, avec preuve de relocation et contexte adéquat. Une valeur inventée, un handler supplémentaire arbitraire ou un thread modèle présenté comme capture ne franchirait pas honnêtement ce blocage.

`Unknown != 0` — `Guest address != host pointer` — `CafeGuestImage != live Cafe process` — `Manmenmi model != reference capture` — `Handler implemented != Cafe semantics proven` — `RUNTIME_EXECUTABLE != REAL_GAME_CONTEXT_MATCH` — `Progress stops where evidence stops`.
