# WOOMY — SEMANTIC V3 : RECONSTRUCTION INTERPROCÉDURALE

Rapport du **4 octobre 2026**. Le socle v3 est implémenté et analysé sur les **115 fonctions**. Il suit arguments, retours, provenance et définitions de stockage, reconstruit des candidats conservateurs et applique un contrôle distinct avant toute promotion machine. **Aucune nouvelle cible n'est prouvée dans le corpus réel : les 41 sites historiques restent non résolus. Le jeu ne démarre pas.** M10 et le dump original restent inchangés.

La [baseline M10](STATUS_M10_BASELINE.md) et la [documentation Semantic v2](SEMANTICS_V2_BASELINE.md) sont conservées. [SEMANTICS.md](SEMANTICS.md) décrit les règles et bornes. Les rapports issus du jeu restent locaux sous `Splatoon/Games/Splatoon/.woomy/reports/`, exclus du dépôt ; aucun extrait du dump n'est incorporé aux tests.

## A. Reproduction de la baseline

Avant modification : **140 tests réussis, zéro échec, trois ignorés**, deux tests externes M10 exécutés séparément et réussis ; formatage, Clippy et release réussis. Les campagnes 512 PPC/IR, 5 120 natives scalaires, 6 144 multi-fonctions et 4 × 512 fonctions réelles en domaine synthétique concordent. Les rapports image, liaison, sémantique globale/entrée, frontière, deux profils, fermeture, indirects et couverture correspondent à M10. Les 167 lignes historiques d'indirects, retours compris, sont inchangées. Journaux : `target/semantic-v3-baseline.{json,log}` et rapports `v3-baseline-*`.

## B. Fichiers et composants modifiés

- `crates/core/src/semantic_values.rs` : valeurs abstraites et mémoire au point d'usage.
- `semantic_interprocedural.rs` : résumés, propagation, graphe de provenance, SCC et contrôle des candidats machine.
- `semantic_reconstruction.rs` : contraintes de types, agrégats, stockage, pointeurs, callbacks, objets, vtables, classes et appels virtuels candidats.
- `semantic_proof.rs` : redérivation, abaissement prouvé et transaction de mise à jour des dépendances.
- `semantic_reporting.rs`, `semantic.rs`, `lib.rs` : compteurs, intégration, produits versionnés et audit exhaustif.
- `crates/ir/src/lib.rs`, `execution.rs`, `crates/codegen/src/lib.rs` : préservation du type indirect de l'événement de trace après promotion statique.
- `crates/core/tests/semantic_v3.rs` : 21 tests synthétiques ; README, CONTEXT, SEMANTICS, SOURCES et présent rapport.

Aucune migration SQLite, dépendance externe ni modification de MANMENMI. Les schémas IR et ABI existants sont conservés.

## C. Architecture Semantic v3

Typed IR courant et vérifié → valeurs/effets locaux → résumés et contraintes interprocédurales → graphe de provenance → reconstruction sémantique. Un chemin séparé redérive les faits machine et contrôle chaque cible avant de modifier l'IR. Une hypothèse de signature ou de classe ne peut pas alimenter directement cette preuve.

Produits : `SEMANTIC` par fonction, `SEMANTIC_TABLES`, `SEMANTIC_INTERPROCEDURAL` et, seulement en cas de promotion, `MACHINE_PROOF`. Rapports `semantic-summary.json`, `semantic-interprocedural.json`, `semantic-indirect-audit.json`. La commande `semantic <fonction>` reste locale ; `semantic` calcule le corpus complet.

Versions : Semantic **3**, interprocédural/provenance/proof gate **1/1/1**, IR **3**, CFG **4**, dataflow **5**, résumé machine **2**. L'identité inclut versions, hashes IR triés, image, contenu mémoire, initialisations et politique d'entrée. Un produit sémantique sauvegardé, même corrompu, n'est pas accepté comme preuve : il est recalculé.

## D. Point fixe interprocédural

Ordre déterministe par fonction et bloc ; point fixe CFG avec jointures de tous les prédécesseurs, puis redérivation des résumés à partir du tour précédent. Les cycles sont recensés par SCC. Les contraintes de types conservent des ensembles monotones de racines dédupliquées ; les résumés de valeurs sont un calcul borné par redérivation, sans prétendre que chaque champ constitue un treillis monotone.

Résultat réel : **six tours interprocéduraux**, convergence globale ; **deux tours de types**, convergence ; propagation des agrégats convergée. 115 SCC, aucune multi-fonction dans ce graphe connu. La récursion et la récursion mutuelle sont testées synthétiquement. Aucun épuisement du budget CFG dans le résultat final. 25 fonctions portent néanmoins une incomplétude : 24 ont des effets source/relocations inconnus, 13 des frontières CFG non prises en charge, avec recouvrement.

Bornes : 128 tours, huit alternatives et 128 définitions par valeur, 4 096 emplacements mémoire, 262 144 nœuds et 524 288 arcs, 4 096 nœuds par chaîne de preuve. Le dépassement reste UNKNOWN ou produit une erreur explicite. L'élargissement pour épuisement de budget est absorbant, afin d'éviter une oscillation artificielle.

## E. Extensions des résumés

Lectures avant définition, arguments candidats observés, registres définis sur les retours connus, modifications possibles, retours candidats, lectures/écritures mémoire, appels et indirects sont persistés. Les effets transitifs utilisent les résumés machine existants. Source-level return type et signature ABI demeurent UNKNOWN. Les fonctions incomplètes ne donnent pas de preuve de cible complète.

## F. Propagation des arguments

221 arguments candidats observés, 618 contraintes de passage, 219 groupes de valeurs entrantes et 91 sujets avec contraintes de types. Les registres r3–r10 sont candidats lorsqu'un usage est observé, pas simplement parce qu'ils existent à l'entrée. Les valeurs d'appel sont liées aux origines chez le callee ; les contraintes de types peuvent revenir vers l'appelant. Les désaccords gardent toutes les alternatives : **11 sujets conflictuels**, sans vote par nombre de passages dans un cycle.

Les valeurs issues des appelants décrivent les appels connus ; elles ne prouvent pas que tous les appelants possibles ont été découverts et ne remplacent pas universellement une entrée symbolique de fonction.

## G. Propagation des retours

18 fonctions présentent des valeurs de retour candidates avec une interprétation typée dans le domaine v3. Les expressions symboliques de callee sont substituées par les arguments du site appelant. Les sorties inconnues et retours discordants restent explicites. Une écriture r3 n'est pas automatiquement un retour source : les **494 écritures r3** et **51 observations locales de retour v2** restent des mesures distinctes, inchangées. Aucun type de retour source n'est déclaré prouvé.

## H. Graphe de provenance

**26 971 nœuds, 23 287 arcs**, ancrés dans fonction, module, bloc, PC et opération IR. Entrées symboliques, relocations, copies, calculs, lectures, écritures, arguments, retours et jointures conservent leurs prédécesseurs. **25 616 opérations IR distinctes examinées**, sur 25 675 opérations du corpus ; les nœuds auxiliaires d'arguments ne gonflent pas ce dénominateur. Les identifiants et racines sont dédupliqués. Le contrôle final vérifie chaque référence de graphe et chaque hash d'IR courant.

## I. Stockage global et mutable

51 emplacements globaux candidats, dont 50 writable. Une écriture démontrée peut établir une valeur dans une région mutable au point d'usage. Une initialisation du chargeur ne prouve pas sa permanence à une entrée runtime arbitraire. La politique réelle est `UNKNOWN_RUNTIME_ENTRY` ; l'hypothèse explicite d'entrée chargeur est réservée à des analyses/tests identifiés et ne passe pas la promotion du projet.

Le mot `0x10107C24`, initialisé par la relocation ADDR32 373076 vers l'entrée `0x02798514`, reste un **mot de stockage mutable candidat**. Aucun accès consommateur prouvé ne le relie aux indirects connus. Il n'est ni une table immuable, ni une cible d'appel prouvée.

## J. Définitions atteignant une lecture

898 lectures observées : **6 DEFINITELY_ONE, 0 FINITE_SET, 150 UNKNOWN et 742 CLOBBERED**. Le zéro FINITE_SET est un résultat de corpus ; les ensembles finis sont implémentés et testés. Écritures conditionnelles, absence de définition sur un chemin, chevauchements, largeurs, permissions et alias inconnus sont pris en compte. Une écriture ultérieure ne modifie pas rétroactivement une lecture. Les appels inconnus invalident les faits que leurs effets possibles peuvent détruire.

## K. Pointeurs de fonction candidats

Détection implémentée : origine d'adresse explicite, adresse alignée, entrée exacte connue et plage exécutable. Les entiers ressemblant à du code, adresses intérieures, cibles non mappées et provenance perdue par troncature ne suffisent pas. Résultat réel : **0 candidat issu d'un flot de valeur prouvé**. Le mot de relocation isolé reste documenté dans le stockage.

## L. Callbacks candidats

Chaîne prise en charge : pointeur connu passé à un callee → stockage du paramètre → invocation de cette origine. Test positif synthétique ; durée de vie et ownership UNKNOWN. Résultat réel : **0**. Un nom d'import ou une forme d'appel seule ne déclenche pas la classification.

## M. Tables candidates

Un seul run de stockage, un mot de quatre octets ; **0 table d'au moins deux entrées**. Les suites de relocations gardent entrées mixtes, conflits, mutabilité et frontières inconnues. Les adresses et indices finis sont propagés ; indice inconnu et écriture non résolue empêchent une unicité fictive. La preuve d'une frontière de table reste UNKNOWN.

## N. Objets et agrégats

**Six agrégats, 23 observations de champs** dans la reconstruction v3 ; six agrégats et 18 champs restent dans le rapport local comparable v2. Les callees contribuent des champs à une origine appelante exactement identifiée ; le passage de base sans décalage est pris en charge. Des offsets égaux ne fusionnent pas des pointeurs d'origines différentes. Les observations propagées gardent leurs sites et chemins d'appel.

**Zéro objet candidat** dans le corpus. Les identités restent neutres ; noms, taille complète, ownership, durée de vie et layout source ne sont pas prouvés. La propagation générique de bases décalées et une analyse complète de chevauchements d'objets restent à étendre.

## O. Vtables candidates

Le détecteur demande une initialisation explicite du champ zéro par l'adresse d'une table, plusieurs entrées exécutables connues et une chaîne atteignant le slot puis l'indirect. Un motif à deux loads suivi de bctrl est insuffisant. Tests positif fort et négatif faible. **Zéro vtable réelle candidate**. La reconnaissance initialisation/utilisation est actuellement locale à une fonction ; elle n'établit pas une durée de vie d'objet à travers des phases runtime inconnues.

## P. Classes candidates

Le détecteur produit des groupements neutres liés à une vtable et ses méthodes candidates lorsqu'il dispose de cette chaîne. **Zéro classe réelle candidate**. Aucun nom source ni héritage n'est inféré. Reconstruction de constructeurs/destructeurs et héritage : **NOT_IMPLEMENTED**. Ce statut ne doit pas être remplacé par un compteur zéro prétendant une recherche achevée.

## Q. VirtualCall candidates

La reconstruction garde objet, table, slot, cibles candidates et alternatives ; cette classification demeure sémantique. **Zéro candidat réel**, zéro appel virtuel résolu. r3 n'est pas automatiquement `this`, offset zéro n'est pas automatiquement une vtable et bctrl n'est pas automatiquement virtuel.

## R. Contrôle de preuve machine

Il faut une valeur unique, aucune alternative inconnue, une provenance complète, des définitions compatibles, des alias et effets d'appel maîtrisés, une plage exécutable, une entrée exacte connue, une convergence et un transfert pris en charge. La promotion actuelle couvre les appels indirects inconditionnels avec lien. Les ensembles finis sont conservés, mais non promus : l'IR actuel n'a pas de forme de dispatch statique gardé pour ces ensembles. Les sauts indirects sans lien restent bloqués.

Le contrôle repart de l'IR et de l'image actuels. La transaction vérifie le hash IR, conserve la chaîne, modifie la cible de l'appel, le site historique, les références et le graphe ; elle recalcule résumés/contrats et invalide les produits dépendants. La valeur de lien, la capture de cible et le type indirect de l'événement de trace sont conservés. Le projet est réanalysé après promotion, avec une borne de 16 tours.

## S. Audit des 41 sites historiques

Les **41 sites hors retours** sont tous présents dans `semantic-indirect-audit.json`, y compris les quatre sites sans transfert IR pris en charge. Répartition : 27 UNKNOWN_CALL_EFFECT, 1 UNKNOWN_ALIAS, 9 UNKNOWN_VALUE, 4 UNSUPPORTED_PATTERN. Les raisons détaillées, définitions, valeurs et preuves manquantes restent attachées à chaque site ; les 167 lignes historiques, retours compris, sont conservées byte pour byte dans leurs champs.

Toutes les fonctions ci-dessous appartiennent au module `module.Gambit.30647d865809`.

| Fonction, suffixe d'identifiant | PC du site | Classification v3 |
|---|---|---|
| `02797248` | `0x02797A84` | UNKNOWN_CALL_EFFECT |
| `02797C20` | `0x02797CA4` | UNKNOWN_ALIAS |
| `02797C20` | `0x02797CBC` | UNKNOWN_CALL_EFFECT |
| `02797FB4` | `0x0279820C` | UNKNOWN_CALL_EFFECT |
| `028019B8` | `0x028019C4` | UNKNOWN_VALUE |
| `02801A78` | `0x02801A40` | UNKNOWN_CALL_EFFECT |
| `02801AC8` | `0x02801A40` | UNKNOWN_CALL_EFFECT |
| `02801ADC` | `0x02801A40` | UNKNOWN_CALL_EFFECT |
| `02801EBC` | `0x02801ED0` | UNSUPPORTED_PATTERN |
| `0280E070` | `0x0280E09C` | UNKNOWN_VALUE |
| `0280E070` | `0x0280E0B0` | UNKNOWN_CALL_EFFECT |
| `0280E070` | `0x0280E0D0` | UNKNOWN_CALL_EFFECT |
| `0280E070` | `0x0280E134` | UNKNOWN_CALL_EFFECT |
| `0280E070` | `0x0280E194` | UNKNOWN_CALL_EFFECT |
| `0280E9A8` | `0x0280EA8C` | UNKNOWN_CALL_EFFECT |
| `0280E9A8` | `0x0280EB6C` | UNKNOWN_CALL_EFFECT |
| `0280E9A8` | `0x0280EBCC` | UNKNOWN_CALL_EFFECT |
| `02810908` | `0x028109D8` | UNKNOWN_CALL_EFFECT |
| `02810908` | `0x02810AB8` | UNKNOWN_CALL_EFFECT |
| `02810908` | `0x02810B18` | UNKNOWN_CALL_EFFECT |
| `02810908` | `0x02810B5C` | UNKNOWN_CALL_EFFECT |
| `02813E60` | `0x02813F1C` | UNKNOWN_CALL_EFFECT |
| `02817F48` | `0x028177BC` | UNKNOWN_CALL_EFFECT |
| `02817F48` | `0x028177F8` | UNKNOWN_VALUE |
| `02817F48` | `0x02817848` | UNKNOWN_CALL_EFFECT |
| `02817F48` | `0x02817884` | UNKNOWN_CALL_EFFECT |
| `02817F48` | `0x028178D0` | UNKNOWN_CALL_EFFECT |
| `02817F48` | `0x028178F8` | UNKNOWN_CALL_EFFECT |
| `02822600` | `0x028224B0` | UNKNOWN_VALUE |
| `02822600` | `0x0282250C` | UNKNOWN_CALL_EFFECT |
| `028261B8` | `0x028261E4` | UNKNOWN_VALUE |
| `028261B8` | `0x02826238` | UNKNOWN_VALUE |
| `02826B90` | `0x02826BFC` | UNKNOWN_CALL_EFFECT |
| `02827148` | `0x028271D0` | UNKNOWN_CALL_EFFECT |
| `02827460` | `0x028274B0` | UNKNOWN_VALUE |
| `0283091C` | `0x0283094C` | UNKNOWN_VALUE |
| `0283091C` | `0x028309B4` | UNKNOWN_CALL_EFFECT |
| `02C6BC20` | `0x02C6BC74` | UNKNOWN_VALUE |
| `02C83090` | `0x02C83098` | UNSUPPORTED_PATTERN |
| `02C83090` | `0x02C830A8` | UNSUPPORTED_PATTERN |
| `02C89B44` | `0x02C89B88` | UNSUPPORTED_PATTERN |

## T. Nouvelles cibles machine et chaînes de preuve

**Zéro dans le corpus réel.** Aucun saut, appel ou symbole du jeu n'a été assigné manuellement. Il n'existe donc aucune chaîne réelle complète à publier comme résolution réussie.

Les tests synthétiques positifs suivent : origine de relocation → valeur de pointeur → écriture → définition atteignante → lecture → CTR → appel indirect → vérification région exécutable/entrée connue → contrôle machine → cible statique et arête d'appel. Le test projet persiste cette promotion et vérifie les résumés reconstruits, la conservation de son classement après réanalyse et le rejet des produits sauvegardés corrompus. Le test différentiel de promotion compare état complet, sortie et trace PPC/IR/natif ; ces preuves synthétiques ne sont pas comptées dans le résultat du jeu.

## U. Graphe d'appels et fermeture

Inchangés : **459 sites**, dont 353 internes, 70 imports et 36 appels indirects ; 35 symboles importés dans trois bibliothèques. La fermeture de l'entrée comprend 115 fonctions connues, 97 partielles et 51 obstacles de contrôle ; elle reste non fermée. Les 41 sites historiques incluent aussi des transferts qui ne sont pas des appels, d'où un dénominateur différent de 36. Les commandes d'appels et de résumé sont vérifiées pour les 115 fonctions.

## V. Couverture machine avant/après

| Mesure | Avant | Après |
|---|---:|---:|
| Fonctions connues / IR structurellement valide | 115 / 115 | 115 / 115 |
| LIFTED / PARTIALLY_LIFTED / INVALID | 18 / 97 / 0 | 18 / 97 / 0 |
| Instructions / blocs / arêtes CFG | 5 187 / 1 283 / 1 658 | 5 187 / 1 283 / 1 658 |
| Fonctions natives éligibles / compilées ordinaires | 18 / 18 | 18 / 18 |
| Indirects hors retours résolus / non résolus | 0 / 41 | 0 / 41 |
| REAL_GAME_CONTEXT_MATCH | 0 | 0 |
| Captures Cafe réelles / handlers validés par référence | 0 / 0 | 0 / 0 |

Module SHA-256 : `45ecd1ed81a97f46b1c8ae24820cd47fbc55399b50af09bf1f9050fe803308e5`. Image : `77fe959643e3db31964dd743fce074884cdb373b0216edd9b9063076cbb5d273`.

## W. Mesures sémantiques avant/après

Les mesures locales comparables restent identiques. Elles ne sont pas remplacées par des totaux interprocéduraux de définition différente.

| Mesure locale | Baseline v2 | Après v3 |
|---|---:|---:|
| `aggregate_candidates` | 6 | 6 |
| `call_operations` | 459 | 459 |
| `cfg_iterations` | 259 | 259 |
| `class_candidates` | 0 | 0 |
| `field_candidates` | 18 | 18 |
| `function_denominator` | 115 | 115 |
| `function_pointer_candidates` | 0 | 0 |
| `functions_analyzed` | 115 | 115 |
| `functions_with_type_candidates` | 100 | 100 |
| `inter_block_conflicts` | 5 | 5 |
| `inter_block_propagated_facts` | 162 | 162 |
| `machine_operation_denominator` | 25675 | 25675 |
| `machine_proven_indirect_targets_from_semantics` | 0 | 0 |
| `memory_operations` | 1825 | 1825 |
| `object_candidates` | 0 | 0 |
| `observed_r3_writes` | 494 | 494 |
| `platform_import_sites` | 70 | 70 |
| `platform_sites_categorized` | 21 | 21 |
| `return_value_candidates` | 51 | 51 |
| `semantic_indirect_candidates` | 0 | 0 |
| `semantic_operations` | 500 | 500 |
| `signature_candidates` | 110 | 110 |
| `signatures_proven` | 0 | 0 |
| `table_candidates` | 0 | 0 |
| `table_storage_runs_examined` | 1 | 1 |
| `type_candidates` | 119 | 119 |
| `type_conflicts` | 1 | 1 |
| `typed_argument_candidates` | 34 | 34 |
| `typed_signature_candidates` | 27 | 27 |
| `virtual_call_candidates` | 0 | 0 |
| `virtual_calls_resolved` | 0 | 0 |
| `vtable_candidates` | 0 | 0 |

La couche interprocédurale n'était pas implémentée dans la baseline : ses nouveaux compteurs ont un état antérieur **NOT_IMPLEMENTED**, sauf les mesures de corpus déjà données séparément. Valeurs réellement observées :

| Mesure v3 | Résultat |
|---|---:|
| `aggregate_candidates` | 6 |
| `callback_candidates` | 0 |
| `candidate_arguments` | 221 |
| `candidate_return_values` | 18 |
| `class_candidates` | 0 |
| `constructor_destructor_candidates` | NOT_IMPLEMENTED |
| `definitely_one_reads` | 6 |
| `field_candidates` | 23 |
| `finite_set_reads` | 0 |
| `function_pointer_candidates` | 0 |
| `functions_analyzed` | 115 |
| `functions_with_candidate_signatures` | 104 |
| `functions_with_semantic_facts` | 114 |
| `global_storage_candidates` | 51 |
| `incomplete_functions` | 25 |
| `interprocedural_facts` | 618 |
| `machine_proven_indirect_candidates` | 0 |
| `mutable_storage_candidates` | 50 |
| `object_candidates` | 0 |
| `reaching_definition_reads` | 898 |
| `semantic_operations` | 2279 |
| `storage_runs` | 1 |
| `table_candidates` | 0 |
| `type_conflicts` | 11 |
| `typed_candidate_arguments` | 91 |
| `typed_candidate_returns` | 18 |
| `typed_ir_operations_examined` | 25616 |
| `value_provenance_edges` | 23287 |
| `value_provenance_nodes` | 26971 |
| `virtual_call_candidates` | 0 |
| `vtable_candidates` | 0 |

`semantic_operations` v3 compte les observations d'accès et d'appel, y compris inconnues ; ce n'est pas un taux de comportement source compris. Le compteur local de 500 opérations interprétées sur 25 675 opérations machine est conservé. Les 23 champs v3 sont des observations liées à des origines, pas 23 champs source prouvés.

## X. État des $UNDEF

**17 restent UNKNOWN/UNSUPPORTED**, sans patch ni valeur devinée. Relocations : 402 508 au total, 393 068 APPLIED_PROVEN_LOCAL, 9 423 DEFERRED_IMPORT, 17 UNSUPPORTED. La nouvelle sémantique ne transforme pas un symbole indéfini en zéro ou en pointeur plausible.

## Y. Régression M10

Les deux tests externes passent contre le M10 original. ABI runtime/service/thread/memory **1/1/1/1**, provider **2**, transport helper **2**, protocole natif de modèle **1**. Deux contrats natifs restent activés : `coreinit::OSGetCurrentThread` et `coreinit::OSIsDebuggerInitialized`. Aucun service, code source, contrat ou runtime M10 ajouté/modifié.

| Profil CAFE_BOOTSTRAP_MODEL | Instructions | Blocs | Appels M10 par moteur | Écritures | Frontière |
|---|---:|---:|---:|---:|---|
| debugger-absent | 13 | 3 | 2 | 5 | Avant fetch `0x02C6BCF0`, relocation indéfinie `0x02C6BCF2` |
| debugger-present | 19 | 4 | 2 | 5 | Trap `0x02C6BCEC` |

PPC/IR/natif concordent dans les deux profils. Ils restent **INCONCLUSIVE**, sans entrée complète ni validation Cafe de référence. Les rapports des profils sont identiques à la baseline.

## Z. Validation PPC/IR/natif

512/512 PPC–IR, 5 120/5 120 natives scalaires, 6 144/6 144 multi-fonctions et quatre campagnes de 512/512 sur `027967CC`, `02798514`, `02838B10`, `02852BB4`. Ces dernières utilisent des états synthétiques ; elles ne valident pas le contexte réel du jeu. Les chemins natifs générés n'intègrent ni interpréteur PPC/IR, ni JIT, ni fallback. Le décodage de provenance utilisé pour conserver le type d'événement a lieu dans l'analyse/génération, pas dans le C++ exécuté.

## AA. Tests ajoutés

21 tests dans `crates/core/tests/semantic_v3.rs` couvrent : mémoire mutable sans hypothèse d'entrée, définitions au point d'usage, écritures conditionnelles et alias, chevauchements/largeurs/permissions, arguments/retours, identités déterministes, récursion et cycles, racines dédupliquées, budgets absorbants, données périmées, entiers ressemblant à des pointeurs, cibles intérieures/non mappées, ensemble fini sans promotion, import inconnu, agrégats d'origines distinctes, contraintes de types conflictuelles, callback explicite, motifs de vtable faible/fort, promotion différentielle PPC/IR/natif et transaction projet avec produit corrompu. Les fixtures sont entièrement synthétiques.

## AB. Résultats complets des contrôles

**161 tests workspace réussis, zéro échec, trois ignorés** ; les deux tests M10 nécessitant le runtime externe sont exécutés séparément et passent. Formatage, Clippy avec warnings interdits et build release passent. Toutes les campagnes historiques de la section Z passent. Image, link audit, sémantique, couverture, fermeture, indirect audit, statuts natif/runtime, frontière et deux profils ont été relancés. Appels et résumés : les 115 fonctions sont contrôlées.

Journaux : `target/semantic-v3-verified.{json,log}` et `target/semantic-v3-final-checks.{json,log}` ; résultats CLI `v3-final-cli.json`. Un premier lancement externe avait reçu le chemin relatif du helper : corrigé en chemin absolu, le test passe. Une invocation de contrôle `calls` sans identifiant a été rejetée par la CLI : remplacée par les appels correctement paramétrés pour tout le corpus. Ces erreurs de lanceur sont conservées séparément, pas masquées comme des réussites produit.

Le second calcul sémantique complet contrôle l'identité byte pour byte des trois rapports globaux. Les contrôles finaux sont enregistrés dans `v3-final-invariants.json` et `v3-final-comparison.json`.

## AC. Intégrité du dump original

**615 fichiers, 1 761 602 316 octets** dans code/content/meta ; hashes SHA-256 comparés à l'inventaire original. **Zéro fichier modifié, manquant ou ajouté.** Seuls les produits dérivés sous `.woomy` et la copie existante du lanceur Woomy à la racine du projet sont actualisés.

## AD. Intégrité MANMENMI M10

**136 fichiers** vérifiés byte pour byte ; aucun modifié, ajouté ou supprimé. Les **25 fichiers source/en-tête consommés** correspondent au manifeste et à leurs hashes. Répertoire : `C:/Users/yukee/Documents/Manmenmi/Manmenmi-M10-NativeRuntime/Manmenmi`. Aucun vendoring ni réimplémentation du runtime Cafe.

## AE. Intégrité SQLite et produits

Schéma **6**, `PRAGMA integrity_check = ok`, zéro violation de clé étrangère. 167 lignes d'indirects historiques identiques, 115 hashes IR actuels vérifiés et zéro référence de graphe manquante. Aucun produit MACHINE_PROOF n'est créé pour le corpus réel. Les produits locaux v3 sont redérivés ; les changements de version ou de contenu ne sont pas ignorés pour réutiliser une hypothèse.

## AF. Hashes des exécutables et du helper

| Artefact | Taille, octets | SHA-256 |
|---|---:|---|
| Woomy baseline M10 | 11068416 | `39d1897251d2530e65a2bdad53b68838ea04e2b7fb9c39b293f09f085d38833e` |
| Woomy v3, release et copie racine identiques | 11892736 | `dbea19fc07d627ba814f9de57e83f063fd1f884a078e80296357d19088d5e7d4` |
| Helper M10 inchangé | 266240 | `dcd84316e092a3cf56ac03634c00a21db9fde982c7fe073917ca05f7f6291e11` |
| Natif debugger-absent inchangé | 253440 | `431207a53928dd2676c552168d0b53949ebb8e56088825304766a1124e1f7812` |
| Natif debugger-present inchangé | 253440 | `224afd3e28f70aa790ce63385039382902c25a26fcf96f99ef0a460281aca1af` |

Source C++ du helper : `6195b8853f3ad72c9c939b0e396a6efb2553f5d6ba61195f623a65409eec6a9f`. Détails et chemins : `v3-final-executables.json`. Synthèse machine lisible : `v3-final-summary.json`.

L'identité interprocédurale est `6c38707d7143c8dde42683947c27f4c7d5111d0bb76226717e716759673d6572`. Le manifeste de compilation, les artefacts natifs et les rapports locaux conservent leurs propres identités et leur domaine de validation.

## AG. UNKNOWN et limites restantes

Appelants non découverts, effets des imports, adresses runtime, alias non résolus, stockage mutable à l'entrée et 17 $UNDEF restent inconnus. 25 fonctions v3 incomplètes interdisent des preuves non justifiées. Les quatre sites sans transfert IR reconnu restent visibles dans l'audit. Les types, agrégats et classes sont des hypothèses, pas des identités source. Dispatch à ensemble fini, promotion des sauts sans lien, propagation générique des sous-objets décalés, durée de vie des objets, constructeurs/destructeurs et héritage ne sont pas entièrement pris en charge.

## AH. Nouveaux obstacles plateforme démontrés

**Aucun nouvel obstacle plateforme concret attribuable à une API M10 manquante n'a été démontré.** Les effets d'appel inconnus sont des insuffisances de preuve dans Woomy ; ils ne justifient pas l'invention d'un service Cafe. Les frontières d'entrée déjà connues restent inchangées. Aucune progression artificielle au-delà d'une relocation indéfinie n'est revendiquée.

## AI. Prochain jalon recommandé

Étendre le suivi interprocédural de mémoire avec des résumés paramétriques de lecture/écriture et des preuves d'alias conservatrices, afin de préciser les 27 sites bloqués par les effets d'appel. En parallèle de cette direction, compléter les frontières Typed IR des quatre sites non pris en charge et les tests de multi-objets, bases décalées et tables modifiées. Garder les mêmes invariants : origine exacte, preuve complète avant promotion, validation différentielle et M10 gelé. Les captures Cafe peuvent enrichir les preuves sans devenir un prérequis aux améliorations statiques génériques.
