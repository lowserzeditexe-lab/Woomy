# WOOMY — CONVERGENCE NATIVE M10 ET ANALYSE SÉMANTIQUE V2

Jalon vérifié le **3 octobre 2026**. Le chemin de validation natif généré appelle réellement MANMENMI M10. Les deux profils concordent avec PPC/IR jusqu'à leur frontière justifiée. **Le jeu ne démarre pas**, l'entrée demeure partielle et aucune validation Cafe de référence n'est revendiquée. La [baseline de réalignement](STATUS_REALIGNMENT_BASELINE.md) et la [baseline M9](STATUS_M9_BASELINE.md) restent conservées.

## A. Baseline reproduite avant modification

134 tests workspace réussis, zéro échec, deux ignorés ; test externe M9 exécuté séparément avec succès. Fmt, Clippy avec warnings interdits, release et campagnes **512 PPC/IR, 5 120 natives scalaires, 6 144 multi-fonctions et 4 × 512 fonctions réelles en domaine synthétique** passent. Les rapports de sémantique, couverture et profils d'entrée sont identiques ; seul l'historique de lancement change. Les 615 fichiers originaux et les hashes des deux exécutables correspondent à la baseline. Journaux `target/m10-baseline.{json,log}`, rapports locaux `m10-baseline-*`.

Un audit lancé pendant une autre commande Woomy a rencontré le verrou du projet ; relancé séquentiellement, il réussit. Ce rejet de concurrence n'était pas une différence de baseline.

## B. M10 réellement inspecté

Répertoire découvert : `C:/Users/yukee/Documents/Manmenmi/Manmenmi-M10-NativeRuntime/Manmenmi`. Inspection de `docs/integration/WOOMY.md`, `M10_EXPORT.md`, `NATIVE_ABI.md`, `docs/validation/M10.md`, `abi/include/manmenmi/abi/manmenmi_abi.h`, `abi/src/manmenmi_abi.cpp`, registres/services/process Cafe et `tests/abi_tests.cpp`. Les tests ABI upstream sont compilés depuis leur emplacement original et exécutés sans copie ni modification de MANMENMI.

Le runtime consomme **25 fichiers sources/en-têtes**, neuf unités compilées, dont la façade C ABI ajoutée aux huit unités Cafe/core. Pas de téléchargement SDL requis pour ce sous-ensemble. Les empreintes consommées sont dans `.woomy/manmenmi.json` et [SOURCES.md](SOURCES.md).

## C. Contrat réel contre proposition Woomy

La proposition était DESIGN_ONLY. M10 fournit des pointeurs opaques propriétaires/empruntés, pas des handles uint64 à génération. Un seul process par runtime ; pas de destruction individuelle via l'ABI C. Dispatch par bibliothèque/symbole, structs de taille explicite, quatre versions **1/1/1/1**.

L'implémentation ne vérifie pas les champs taille/version des callbacks mémoire et `mnm_query_capability` n'intercepte pas ses allocations possibles. Woomy construit les callbacks exacts et négocie via `mnm_service_count`/`mnm_service_at`, dont le chemin allouant est protégé. Le setter de thread courant reçoit uniquement l'ID créé avec succès dans le même process. Aucune garantie générique de handles périmés ou d'absence d'exception sur toutes les API M10 n'est inventée. Détails : [NATIVE_BRIDGE.md](NATIVE_BRIDGE.md).

## D. Architecture effectivement livrée

`entry run-model` : IR vérifié → sous-graphe borné avant barrières → C++ généré → x86-64 → ABI C M10 → vrai CafeRuntime/CafeProcess/CafeServiceRegistry → continuation native. PPC et IR sont exécutés séparément comme oracles avec leurs propres runtimes M10.

`crates/codegen/src/model.rs` et `model.cpp` constituent la voie dédiée de validation ; `validation.rs` transporte son identité. Le codegen et transport ordinaires refusent toujours les liaisons externes qu'ils ne savent pas transporter. Aucun interpréteur PPC/IR, JIT ou fallback dans le chemin natif. Le fichier C++ dérivé reste sous `.woomy/generated/runtime/model-<identity>.cpp`, et la compilation consomme la chaîne générée en mémoire.

## E. ABI, identité, ownership et durée de vie

ABI C M10 runtime/service/thread/memory **1/1/1/1** ; provider Woomy **2**, transport helper **2**, protocole natif de modèle **1**. Typed IR **3**, backend C++ **3**, Native ABI historique **2**, Runtime ABI **2**, SQLite **6**, process/thread/import semantics **1/1/1** inchangés.

Identité compilée, transmise, comparée et renvoyée : programme et imports, module/hash/image, domaine, descripteur complet et versions, helper, état initial complet, sources du runtime/harness/bridge natif. Le lanceur vérifie aussi le hash du binaire. Cette chaîne conserve la liaison runtime ; elle n'est pas une attestation externe. Runtime possédé par un objet RAII non copiable ; process/thread empruntés et confinés à sa durée de vie. Aucun handle brut ne traverse vers Rust ou le guest. Isolation par instances séparées.

## F. GuestMemory

Une seule mémoire autoritaire par exécution : Machine côté oracles, State Woomy côté natif. M10 n'en conserve aucune copie. Callbacks explicites validate/read/write, adresses uint32, vérifications uint64, permissions, plafond 65 536 octets, buffers temporaires bornés, callbacks `noexcept`. Les écritures natives sont prévalidées/préallouées et enregistrées dans l'ordre. Aucun guest address converti en host pointer, aucune sérialisation brute de struct hôte.

## G. Imports supportés

Deux contrats natifs : `coreinit::OSGetCurrentThread()` et `coreinit::OSIsDebuggerInitialized()`, sans argument, résultat r3. La politique de modèle préserve les autres champs CPU/mémoire ; le dispatcher gère LR. La signature Cafe générale reste UNKNOWN. Les **33 autres identités** parmi les 35 imports de fermeture ne sont pas supportées par cette voie native. Les six services disponibles dans M10 ne sont pas automatiquement activés pour le jeu.

## H–J. Exécutions natives et comparaison complète

| Profil CAFE_BOOTSTRAP_MODEL | Instructions | Blocs parcourus | Appels M10 réussis par moteur | Écritures ordonnées | Arrêt |
| --- | ---: | ---: | ---: | ---: | --- |
| debugger absent | 13 | 3 | 2 | 5 | avant 0x02C6BCF0, relocation `$UNDEF` à 0x02C6BCF2 |
| debugger présent | 19 | 4 | 2 | 5 | trap 0x02C6BCEC |

**H —** Quatre appels natifs M10 réussis sur ces deux campagnes d'entrée, deux par profil. Le sous-ensemble testé reçoit **RUNTIME_EXECUTABLE / CAFE_BOOTSTRAP_MODEL**.

**I —** GPR, bits FPR, CR, LR, CTR, XER, FPSCR, mémoire, écritures ordonnées, trace des PC source, événements d'import, terminaison et liaison runtime sont égaux entre PPC/IR/natif. Le programme synthétique reprend après les appels et vérifie le résultat, y compris des bits FPR de type NaN. Un troisième profil synthétique debugger UNKNOWN échoue explicitement au second appel et conserve la valeur précédente de r3.

**J —** Les deux entrées restent **INCONCLUSIVE**, complete_entry=false, real_game_context_match=false. Le trap n'est pas un retour normal. Thread 0x70000000 et pile restent des entrées MANMENMI_MODEL. Le sous-ensemble exécuté n'autorise aucune promotion de la fonction complète.

## K. `$UNDEF`

**17 UNKNOWN, 0 résolue, 0 patchée.** Aucune preuve nouvelle de liaison rencontrée. Les records, cibles et bytes d'origine sont inchangés. La frontière est respectée avant fetch ; ni zéro, ni trampoline, ni faux import, ni instruction sautée. [LINKING.md](LINKING.md) conserve l'audit détaillé.

## L–M. Sémantique v2 et règles de propagation

**L —** Produits version 2 au-dessus de Typed IR, candidats/préuves persistants, observations entrantes sur CFG, arguments typés candidats et distinction écritures r3/retours candidats. Représentations de types et types réellement inférés sont listés séparément. Aucun type source, nom métier, classe ou ABI générale n'est affirmé.

**M —** Point fixe borné : les faits identiques survivent ; alternatives conflictuelles limitées à quatre ; UNKNOWN d'un prédécesseur domine. Les appels/effets opaques invalident les faits ; aucun cache de valeurs mémoire ne survit à un alias incertain. Les ancres portent fonction via le rapport, bloc/PC/opération, définitions, consommateurs et prédécesseurs. Plafonds : 256 itérations/fonction et 256 ancres/fait. Les **259 itérations** agrégées sont une somme sur 115 fonctions, pas un dépassement d'une fonction. Règles complètes : [SEMANTICS.md](SEMANTICS.md).

## N. Pointeurs de fonction et tables

Analyse ADDR32 appliquée + octets concordants + débuts de fonctions exécutables connus + permissions effectives. Une suite de stockage observée : **0x10107C24**, largeur quatre, relocation **373076**, cible **0x02798514**. Le stockage est writable : **REJECTED_OR_CONFLICTING**, zéro pointeur retenu, zéro table retenue, zéro consommateur/call-site prouvé. Il ne s'agit pas d'une table immuable ni d'une cible runtime démontrée. Les fixtures synthétiques montrent l'acceptation hypothétique d'une table readonly cohérente et le rejet/conflit d'entrées invalides ou mutables.

## O. Objets, vtables et classes

Zéro objet/vtable/classe/appel virtuel candidat ou résolu. Les détecteurs complets ne sont pas implémentés. Le motif deux loads→CTR→bctrl est testé comme insuffisant. Les opérations FunctionPointerLoad/TableLookup/VirtualCall/AddressOf/ArrayAccess ne sont pas émises sans preuve du flot requis ; les consommateurs de relocation recensés ne prouvent pas ce flot. Constructeurs/destructeurs et identité source restent UNKNOWN.

## P. Couverture machine avant → après

| Mesure | Avant | Après |
| --- | ---: | ---: |
| Fonctions connues | 115 | 115 |
| LIFTED / PARTIALLY_LIFTED / INVALID | 18 / 97 / 0 | 18 / 97 / 0 |
| Instructions / blocs / edges | 5 187 / 1 283 / 1 658 | 5 187 / 1 283 / 1 658 |
| Natives éligibles / compilées ordinaires | 18 / 18 | 18 / 18 |
| Relocations appliquées / différées / unsupported | 393 068 / 9 423 / 17 | 393 068 / 9 423 / 17 |
| Obstacles de contrôle de fermeture | 51 | 51 |
| Sites d'import / symboles distincts | 70 / 35 | 70 / 35 |
| Contextes réels prêts / validés | 0 / 0 | 0 / 0 |

La fermeture reste ouverte : 115 fonctions connues, 97 partielles ; 459 sites d'appel, dont 353 internes, 70 importés et 36 indirects. Les quatre opcodes UNKNOWN sont inchangés. Les 18 fonctions éligibles conservent leurs contextes de référence incomplets (4 register-only, 14 mémoire).

## Q. Mesures sémantiques avant → après

| Mesure | v1 | v2 |
| --- | ---: | ---: |
| Fonctions analysées | 115 | 115 |
| Fonctions avec types | 104 | 100 |
| Types candidats | 810 | 119 |
| Conflits de types | 1 | 1 |
| Faits inter-blocs consommés | 0 | 162 |
| Conflits inter-blocs | 0 | 5 |
| Signatures candidates | 107 | 110 |
| Signatures avec arguments typés candidats | 0 | 27 |
| Arguments typés candidats | 0 | 34 |
| Agrégats candidats | 91 | 6 |
| Champs candidats | 244 | 18 |
| Objets candidats | 0 | 0 |
| Pointeurs de fonction retenus | 0 | 0 |
| Tables retenues | 0 | 0 |
| Vtables candidates | 0 | 0 |
| Classes candidates | 0 | 0 |
| Appels virtuels candidats | 0 | 0 |
| Imports classés | 21 | 21 |
| Opérations sémantiques | 1036 | 500 |

Dénominateurs inchangés : **115 fonctions, 25 675 opérations IR, 1 825 accès mémoire, 70 sites importés**. La diminution des types/agrégats/champs est attendue : v2 retire les bases symboliques fraîches que v1 supposait à chaque bloc et refuse de transmettre une origine à travers les appels opaques/prédécesseurs inconnus. Elle ne réduit pas la couverture machine. **494 écritures r3 observées, 51 candidats de retour**, zéro type de retour source prouvé ; zéro signature/paramètre ABI prouvé. Les nouveaux compteurs absents de v1 sont notés zéro comme capacité non livrée.

## R. Indirects

**0 résolu / 41 non résolus → 0 / 41** hors retours. Les **167 lignes** historiques, retours inclus, sont identiques colonne par colonne. Zéro candidat indirect sémantique promu ; la barrière de preuve machine demeure fermée.

## S. Tests ajoutés et renforcés

Cinq tests sémantiques supplémentaires couvrent propagation linéaire/provenance, diamant identique/conflit/inconnu, boucle et appel opaque, alias/motif virtuel insuffisant/scalaires, tables readonly/mutables/mixtes/absence de relocation. Les tests existants de persistance, sérialisation, stale IR et propriété sur 128 paires d'offsets sont conservés.

Un test natif ordinaire couvre identité module/image/provider/état, binding incohérent, signature non supportée, maintien des refus historiques. Un test externe M10 compile/exécute la suite ABI upstream, un probe des callbacks natifs (bornes, permissions, overflow, buffers, ordre, noexcept), les trois profils synthétiques et le rejet d'une identité de transport altérée. Le test externe provider existant couvre deux runtimes isolés, état inconnu, mappings manquants, hashes, copies bornées et erreurs. Les handles nulls, version incorrecte et cycles create/destroy sont testés par upstream ; un pointeur libéré n'est jamais déréférencé pour simuler un handle périmé.

## T. Validation finale

**140 tests workspace réussis, 0 échec, 3 ignorés**. Deux tests externes requis exécutés séparément et réussis ; le troisième ignoré est le test historique préexistant. Dernière campagne ciblée native : deux tests réussis avec `--include-ignored`, intégrant le rejet de transport altéré. Fmt/Clippy `-D warnings`/release réussis.

Validateurs : **512 / 512**, **5 120 / 5 120**, **6 144 / 6 144**, **4 × 512 / 512** MATCH, zéro mismatch. Dix-sept commandes finales réussies : audit, image, sémantique globale/entrée, analyze, frontier, deux profils, closure, indirect, couverture, statuts natifs entrée/éligible, runtime, status et aides. Le smoke test de l'exécutable copié confirme l'aide M10.

Journaux `target/m10-verified.{json,log}` et `target/m10-final-metadata.{json,log}`. Les premiers essais ont exposé une continuation de bloc oubliée dans la représentation coupée, un mauvais marqueur d'insertion C++, puis deux fonctions inutilisées dans le probe C++ compilé avec /WX ; corrigés sans abaisser les vérifications. Une invocation exploratoire `native status` sans ID a été corrigée. Aucun résultat divergent n'a été masqué ni attente historique modifiée.

## U. Intégrité du dump

Avant/après : **615 fichiers, 1 761 602 316 octets, 0 modifié, 0 manquant, 0 inattendu** sous code/content/meta. SQLite integrity_check=ok, zéro violation de clé étrangère, schéma 6. Les 115 produits sémantiques ont leurs hashes IR, ancres et références vérifiés. Les dérivés restent sous `.woomy` ignoré ; aucune source de jeu dans les tests.

## V. Intégrité MANMENMI

**136 fichiers du répertoire M10 inchangés**, aucun ajouté/manquant/modifié. Les 25 sources/en-têtes consommés correspondent au manifeste. Le snapshot complet `m10-sources-before.json` couvre aussi les sources de tests et la documentation. Aucun vendoring, fork ou édition upstream. Les objets runtime restent implémentés en C++ MANMENMI.

## W. Empreintes exécutables

| Artefact | SHA-256 |
| --- | --- |
| Woomy release et copie racine jeu, **11 068 416 octets** | `39d1897251d2530e65a2bdad53b68838ea04e2b7fb9c39b293f09f085d38833e` |
| Helper provider M10 | `dcd84316e092a3cf56ac03634c00a21db9fde982c7fe073917ca05f7f6291e11` |
| Source adaptateur Woomy | `6195b8853f3ad72c9c939b0e396a6efb2553f5d6ba61195f623a65409eec6a9f` |
| Préfixe natif debugger absent, 253440 octets | `431207a53928dd2676c552168d0b53949ebb8e56088825304766a1124e1f7812` |
| Préfixe natif debugger présent, 253440 octets | `224afd3e28f70aa790ce63385039382902c25a26fcf96f99ef0a460281aca1af` |

La copie de Woomy à la racine du dump a été remplacée atomiquement après vérification du hash précédent et des chemins ; code/content/meta sont intacts. Les hashes et chemins détaillés sont dans `m10-executables.json`. Aucun commit/staging créé.

## X. UNKNOWN et classes de preuve

- **FACT** : builds/tests/empreintes/comparaisons et chemins réellement exécutés consignés.
- **DOCUMENTED** : header et documentation M10 décrivent le contrat ; les limites effectives priment sur une garantie textuelle trop large.
- **PUBLIC_IMPLEMENTATION_OBSERVATION** : déclarations WUT/ABI et travaux de liaison antérieurs ; aucune nouvelle preuve publique de `$UNDEF`.
- **EMPIRICALLY_OBSERVED_IN_LOCAL_DUMP** : instructions, relocations, permissions et emplacement 0x10107C24.
- **WOOMY_INFERRED** : types/signatures/agrégats/faits CFG, hypothèses sans identité source prouvée.
- **MANMENMI_MODEL** : processus/thread/pile/debugger explicites, **deux handlers activés** parmi six implémentés upstream.
- **SYNTHETIC_VALIDATED** : deux contrats traversés nativement, trois profils synthétiques et deux préfixes d'entrée en contexte de modèle.
- **REFERENCE_VALIDATED** : **0 handler, 0 contexte réel, 0 REAL_GAME_CONTEXT_MATCH**.
- **UNKNOWN** : liaison `$UNDEF`, état Cafe réel, ABI/clobbers généraux, objets/classes/vtables, scheduling et entrée complète.

## Y. Prochaine frontière fondée sur les preuves

Pour l'entrée, obtenir une règle producteur/chargeur, un link map authentique ou une observation légitime du record `$UNDEF` exact ; aucun service supplémentaire ne remplace cette liaison. Indépendamment, approfondir les définitions de valeurs chargées et leur flot vers CTR/LR avec une preuve de mutabilité/alias, puis les signatures/retours inter-fonctions. Le slot writable 0x10107C24 fournit une piste de stockage, pas une cible exécutable démontrée. Les futures API M10 devront préciser la validation des callbacks et le comportement des handles/erreurs avant d'élargir la frontière native.

Rapport machine local complet : `.woomy/reports/m10-final-summary.json` ; détails dans `m10-final-*`, `m10-invariants.json`, `m10-integrity-after.json`, `semantic-tables.json` et les produits SQLite.

**Unknown != 0 — Guest address != host pointer — CafeGuestImage != live Cafe process — Manmenmi model != reference capture — Handler implemented != Cafe semantics proven — RUNTIME_EXECUTABLE != REAL_GAME_CONTEXT_MATCH — Compiled != semantically validated — Semantic inference != proven source-level identity — Confidence != evidence — PPC execution in Woomy = analysis/validation mechanism — PPC execution in final product = NOT THE TARGET ARCHITECTURE — Progress stops where evidence stops.**
