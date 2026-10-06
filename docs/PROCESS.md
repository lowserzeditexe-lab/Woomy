> Mise à jour M10 : le provider courant consomme l'ABI C M10 1/1/1/1, transport 2, provider 2. `runtime connect` attend la racine M10 contenant `abi/`, `cafe/` et `core/`. `entry run-model` compare désormais PPC/IR/natif dans le modèle explicite. Les descriptions M9 ci-dessous documentent l'origine inchangée des objets et des valeurs du modèle ; voir [NATIVE_BRIDGE.md](NATIVE_BRIDGE.md) pour le contrat actuel et ses différences.

# Processus Cafe et intégration de MANMENMI M9

Woomy consomme le runtime C++ existant fourni dans `Manmenmi-M9-CafeRuntime`. Il ne recrée pas CafeRuntime en Rust et ne modifie pas les sources M9. La connexion compile huit unités `core/cafe` avec un adaptateur Woomy C++20 ; elle ne requiert pas SDL, GPU, audio ou le build complet de M9.

## Répartition des responsabilités

| Woomy | MANMENMI M9 |
| --- | --- |
| Image dérivée, CPU PPC/IR, GuestMemory, permissions, dispatch, limites, validation | CafeRuntime, CafeProcess, CafeThread, registre d'objets et services coreinit |
| `CafeProcessDescriptor` : entrées sérialisables, provenance, stockage guest initial | Création effective du processus et de ses threads, sélection du thread courant, état debugger |
| `RuntimeProvider` externe et adaptation des résultats vers r3 | Invocation effective de `services().invoke()` |

`woomy-manmenmi` est un adaptateur. Ses structures Rust ne sont pas une seconde implémentation des services Cafe. Un processus auxiliaire héberge M9 ; chaque exécution PPC/IR possède sa propre instance. Les threads hôtes de transport ne représentent pas des OSThread. Aucun ordonnanceur Cafe n'est implémenté.

La mémoire guest appartient exclusivement à Woomy. La vue mémoire C++ M9 appelle le transport validate/read/write, qui consulte le même `Machine`. Le helper ne conserve pas de copie de RAM. Adresses et tailles sont des entiers guest 32 bits, jamais des pointeurs hôtes. Champs de transport encodés individuellement en little endian ; objets guest matérialisés en big endian. Aucun layout de structure hôte ne traverse cette frontière.

## Connexion et identités

```powershell
woomy runtime connect 'C:\Users\yukee\Documents\Manmenmi\Manmenmi-M9-CafeRuntime'
woomy runtime process
woomy runtime thread debugger-absent
woomy runtime imports
woomy entry frontier
woomy entry run-model debugger-absent
woomy entry run-model debugger-present
```

`runtime connect` compile uniquement l'adaptateur et les unités nécessaires dans `.woomy/generated/runtime`. `.woomy/manmenmi.json` conserve 22 empreintes de sources/en-têtes, l'empreinte de l'adaptateur, celle de l'exécutable, les versions et le compilateur. Les sources sont revérifiées après compilation et avant utilisation. Le handshake confirme protocole/runtime/thread/service ABI **1/1/1/1**.

Versions Woomy : process **1**, thread model **1**, provider ABI **1**, import semantics **1**, runtime ABI **2**. Les identités lient versions, module/image, domaine, état initial, thread courant et empreinte de l'exécutable M9. Programme, Machine et provider doivent porter la même liaison. Une absence ou différence arrête avant exécution. Le changement runtime ABI invalide les anciens artefacts natifs et résultats synthétiques courants ; les anciens rapports sont conservés. SQLite reste au schéma 6.

Les limites comprennent 1 Mio de mémoire exécutable par Machine, 512 instructions, 32 appels, profondeur 16 et 128 événements ; jusqu'à 32 threads et 256 régions sont acceptés dans un descripteur. Le transport borne trames, callbacks, écritures et délais. Un défaut de protocole invalide le canal ; un service indisponible ne renvoie pas un succès zéro.

## Modèle explicite, valeurs inconnues et objets

Sans profil, `runtime process` expose les valeurs inconnues comme `null`. `debugger-absent` et `debugger-present` choisissent explicitement un domaine **CAFE_BOOTSTRAP_MODEL**. Ils ne décrivent pas un processus réel capturé.

| Entrée du modèle | Valeur choisie |
| --- | --- |
| Thread courant / objet guest | `0x70000000`, taille `0x6A0`, alignement 8 |
| Pile | `[0x70010000, 0x70020000)`, r1 initial `0x7001FF00` |
| Core | 1, choix du modèle |
| Debugger | false ou true selon le profil explicite |
| Autres GPR, FPR, CR, CTR, XER, FPSCR | zéro explicitement fourni par le modèle |
| LR initial | `0xFFFF0003`, sentinelle de retour du harnais |

Ces plages sont contrôlées contre toutes les régions réservées de l'image. Seuls code d'entrée, pile et stockage thread sont mappés dans l'exécution ; les données globales du jeu ne deviennent pas implicitement de la RAM vivante connue.

La matérialisation du thread initialise explicitement un backing de modèle à zéro, puis écrit des champs big endian contrôlés : tag OSContext à 0, tag OSThread à `0x320`, état à `0x324`, identifiant à `0x326`, bornes de pile à `0x394/0x398`, entrée à `0x39C`. Le layout vient d'observations WUT ; les valeurs viennent du modèle. Les autres octets zéro ne constituent aucune preuve de sémantique Cafe. L'écriture initiale est prévalidée en totalité et ne compte pas comme une écriture du programme.

Un objet capturé conserve exactement ses chunks et leurs provenances ; les trous restent non mappés, sans zero-fill implicite. Le constructeur de stockage thread actuel utilise des régions RW : il n'infère pas les protections d'une capture. Le replay de snapshots conserve les permissions explicitement enregistrées. Les valeurs `ReferenceCapture` ne sont pas acceptées dans un domaine de modèle et réciproquement. Un label de provenance ne prouve pas à lui seul l'authenticité externe.

## Services et contrats

M9 annonce six services disponibles/model implemented. Woomy n'autorise sur le chemin du jeu que **OSGetCurrentThread** et **OSIsDebuggerInitialized**. Les quatre autres ne sont pas le prochain obstacle et ne sont donc pas activés pour ce préfixe.

- `OSGetCurrentThread()` : M9 retourne l'adresse guest associée au thread courant. Dans ce modèle, r3 reçoit `0x70000000`. Sans thread sélectionné ou avec mapping manquant, échec explicite.
- `OSIsDebuggerInitialized()` : M9 retourne le booléen explicitement fourni, dans r3. Un état inconnu est une erreur, pas false.

Le contrat de modèle ne modifie que r3 ; le dispatcher gère LR. Il ne lit/écrit pas de RAM guest pour ces deux requêtes. Cette préservation des autres registres est une décision du modèle, pas une preuve de l'ensemble volatile/nonvolatile Cafe. Effets internes, synchronisation, timing, exceptions et ABI Cafe complète restent UNKNOWN. Les résumés statiques du jeu ne sont pas promus à KNOWN. **2 MODEL_IMPLEMENTED, 0 REFERENCE_VALIDATED**.

Les tests de transport appellent aussi les implémentations M9 memcpy/OSBlockMove/OSBlockSet sur mémoire synthétique : limites, permissions, overlap pour OSBlockMove et ordre des écritures. Cela n'active pas ces services dans le provider du jeu et ne valide pas leur comportement Cafe réel.

## Frontière d'entrée observée

Module local `module.Gambit.30647d865809`, entrée `0x02C6BCA4`. L'analyse statique énumère les chemins possibles sans choisir un état debugger. Les appels importés ne sont acceptés que sur sites REL24 avec liaison prouvée. Toute autre relocation source non résolue est une barrière avant lecture/exécution de l'instruction.

| Situation | Instructions exécutées/franchissables | Blocs terminés | Appels réussis | Arrêt |
| --- | ---: | ---: | ---: | --- |
| Avant provider, prérequis mémoire supposés | 8 (statique) | 0 | 0 | OSGetCurrentThread à `0x02C6BCC4` |
| Modèle debugger absent | 13 (PPC et IR) | 3 | 2 | avant `0x02C6BCF0` : relocation `$UNDEF` à `0x02C6BCF2` |
| Modèle debugger présent | 19 (PPC et IR, trap inclus) | 4 | 2 | trap à `0x02C6BCEC` |

Les deux parcours réalisent cinq écritures de pile ordonnées. PPC et IR concordent sur état complet, trace et événements, mais le résultat reste **INCONCLUSIVE** : ni l'entrée complète, ni un contexte réel n'ont été validés. Le chemin explicite de validation M10 compare aussi le natif ; codegen et transport ordinaires continuent de refuser l'abandon de cette identité runtime.

L'instruction ultérieure `addi r3,r29,0x408` forme une adresse dans le thread ; elle n'établit pas la signification d'un champ à cet offset et n'est pas atteinte par le préfixe sans debugger. Le prochain fait manquant est la liaison et la valeur légitime de `$UNDEF`, pas un nouvel import à remplacer par un stub. Aucun changement aux quatre opcodes UNKNOWN ni aux 41 indirects historiques.

## Capture future et replay

Aucune référence Cafe réelle n'est disponible dans les artefacts de ce projet. La capture externe et son attestation restent à fournir ; aucune capture Cemu/Wii U n'a été fabriquée.

Pour une capture légitime : arrêter exactement à l'entrée de fonction, consigner origine, outil/version, procédure reproductible, hashes module/image et placement effectif ; enregistrer les 32 GPR/FPR en bits, CR/LR/CTR/XER/FPSCR/CIA et les régions nécessaires avec adresses, permissions et octets exacts. Inclure pile, thread et globals réellement utilisés, sans compléter les trous. Pour les services Cafe, fournir aussi état process/thread/debugger, preuve de leur association et observation des effets avant/après chaque appel. Une capture à mi-bloc n'est pas supportée. Si le placement diffère de l'image courante, il faut établir une nouvelle image justifiée, pas changer seulement son hash.

Le JSON snapshot v1 se place sous `.woomy/snapshots`, avec `ReferenceSnapshot` pour toute valeur d'une référence. `snapshot inspect`, `snapshot validate`, puis `snapshot replay` vérifient identité, bornes, versions et octets exécutables. Replay exécute PPC/IR et, si éligible, un binaire natif fraîchement compilé, budget 512. L'empreinte de sortie facultative est SHA-256 du JSON compact serde de l'`Execution` PPC (état complet, écritures, trace, terminaison), au schéma de runtime courant. Une divergence donne REPLAY_MISMATCH ; absence du natif ou arrêt incomplet donne INCONCLUSIVE. Une concordance complète donne REPLAY_EXECUTION_AGREEMENT tout en conservant l'origine, sans attestation automatique ni REAL_GAME_CONTEXT_MATCH.

Le replay snapshot ne transporte pas encore de sidecar process M9 : les captures nécessitant ces imports restent bloquées. Il est opérationnel pour le sous-ensemble de fonctions dont l'état enregistré suffit. Un snapshot SYNTHETIC reste synthétique après replay.

## Invariants

`Unknown != 0`; `Guest address != host pointer`; `CafeGuestImage != live Cafe process`; `Manmenmi model != reference capture`; `Handler implemented != Cafe semantics proven`; `RUNTIME_EXECUTABLE != REAL_GAME_CONTEXT_MATCH`; `Progress stops where evidence stops`.

## Portée après réalignement

Ce bootstrap PPC/IR reste une sonde de validation. La [direction officielle](DIRECTION.md) vise une reconstruction et une exécution natives ; le [bridge natif M10](NATIVE_BRIDGE.md) implémente désormais le sous-ensemble de validation. Le résultat de l’[audit `$UNDEF`](LINKING.md) ne change ni les valeurs du modèle ni les deux frontières observées.

## Frontière conservée avec Semantic v4

Les effets mémoire v4 sont des inférences statiques sur le Typed IR courant. Ils ne complètent pas l'état d'un process Cafe et ne transforment pas l'image initiale writable en mémoire runtime constante. Le modèle reste UNKNOWN_RUNTIME_ENTRY. Les deux contrats natifs M10 restent des MANMENMI_MODEL ; tous les autres imports conservent leurs effets inconnus. La branche conditionnelle vers LR ajoute une représentation machine, sans fournir la valeur de LR ni supprimer les blocages de contexte/fermeture.
