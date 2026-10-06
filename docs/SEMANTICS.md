# Analyse sémantique v4

Typed IR 4 reste autoritaire. Types, signatures, agrégats et classes sont des hypothèses du domaine **STATIC_MACHINE_IR_HYPOTHESES**. La baseline v3 est conservée dans [SEMANTICS_V3_BASELINE.md](SEMANTICS_V3_BASELINE.md) et [STATUS_V3_BASELINE.md](STATUS_V3_BASELINE.md). Les mesures courantes figurent dans [STATUS.md](STATUS.md).

## Produits et identités

`semantic` redérive les produits locaux et interprocéduraux de toutes les fonctions courantes. `semantic <function-id>` conserve la vue locale détaillée. La vue interprocédurale complète se trouve dans les rapports globaux :

- `reports/semantic-summary.json` : métriques locales historiques, `v3_metrics`, `v4_metrics` et promotions persistées.
- `reports/semantic-interprocedural.json` : résumés, contraintes, graphe de provenance, SCC, stockage et reconstruction par module.
- `reports/semantic-indirect-audit.json` : tous les sites historiques non-retour, classifications et preuves manquantes.
- `reports/semantic-tables.json` : observations de la passe de relocations préexistante.

SQLite 6 reçoit SEMANTIC, SEMANTIC_TABLES, SEMANTIC_INTERPROCEDURAL et, seulement après promotion, MACHINE_PROOF. Aucun produit sémantique enregistré n'est consommé comme vérité machine. Identités : module/hash, image/hash mémoire, hashes des IR participants, versions IR/CFG/dataflow/résumé machine, Semantic 3, modèle interprocédural 1, provenance 1 et contrôle de preuve 1. L'hypothèse d'entrée mémoire est incluse. Les entrées incohérentes ou périmées sont refusées ; les produits sont recalculés.

## Propagation

La passe locale conserve les règles v2 et leurs compteurs comparables. La nouvelle passe interprète abstraitement les valeurs et le stockage. Les états des blocs sont réévalués au point fixe, puis les résumés et contraintes d'appel sont réévalués ensemble. Les SCC sont conservées ; récursion et récursion mutuelle restent bornées, sans déroulage de chemins.

Les résumés conservent lectures avant définition, arguments candidats observés, registres définis sur les retours connus, modifications possibles, effets mémoire locaux/transitifs, accès de stockage et appels. Les effets transitifs viennent des résumés machine existants, qui ne sont pas remplacés. Les entrées r3–r10 ne deviennent candidates qu'après usage local ou usage observé chez un callee connu. Cela ne prouve pas une ABI Cafe complète.

Les expressions de retour d'un callee sont substituées par les valeurs du site appelant. Une écriture r3, une valeur atteignant un retour et un type de retour source restent distincts ; le dernier reste UNKNOWN. Les contraintes de types circulent dans les deux directions lorsque l'origine est compatible. Leurs racines sont des ensembles monotones dédupliqués : un cycle ne multiplie pas les preuves. Les incompatibilités conservent toutes les interprétations.

## Provenance et stockage

Une valeur conserve alternatives bornées, définitions et raisons d'incomplétude. La provenance d'adresse est séparée du nombre brut. Copies, additions 32 bits, relocations et retours connus sont propagés ; les opérations non prises en charge restent inconnues. Le graphe conserve fonction/module, bloc, PC, opération IR, transformation, valeur et prédécesseurs. Arguments, retours, jointures, entrées et initialisations sont explicites ; les accès décrivent stockage global, pile et champs.

Les écritures dominantes peuvent établir une valeur dans une région writable. Les jointures gardent toutes les définitions possibles ; un chemin inconnu interdit une unicité fictive. Alias inconnus, effets d'appel, chevauchements et largeurs différentes invalident les réutilisations non justifiées. Une écriture étroite tronque la valeur et perd la provenance de pointeur. Les offsets symboliques suivent l'arithmétique modulo 32 bits. Une écriture postérieure ne change pas rétroactivement une lecture précédente.

Les lectures portent DEFINITELY_ONE, FINITE_SET, UNKNOWN ou CLOBBERED et leurs raisons. Plages et permissions sont contrôlées. Une mémoire exécutable writable interdit la promotion dans ce modèle statique. La commande projet utilise toujours UNKNOWN_RUNTIME_ENTRY : une relocation mutable du chargeur ne prouve pas sa valeur à une entrée quelconque. EXPLICIT_LOADER_ENTRY est une hypothèse explicite de test incluse dans l'identité ; elle ne passe pas le pont de promotion du projet.

## Reconstruction

Un pointeur de fonction exige une origine d'adresse, une entrée connue et une région exécutable. Un entier ressemblant à du code ne suffit pas. Un callback candidat exige une chaîne valeur passée → stockée → invoquée ; son nom ne suffit pas et sa durée de vie reste UNKNOWN.

Les suites de relocations gardent les entrées mixtes/non résolues. Un mot isolé n'est pas compté comme table. Les lecteurs/écrivains et valeurs atteignant les lectures sont séparés de l'initialisation. Les adresses constantes et ensembles finis d'indices sont suivis ; un indice inconnu demeure inconnu.

Les champs des callees peuvent être instanciés à une origine exactement identifiée chez l'appelant. La propagation d'identité compose maintenant les déplacements constants prouvés, y compris à travers plusieurs appels ; elle ne transforme pas un offset en identité de sous-objet source. Des offsets similaires ne fusionnent pas des origines distinctes. Les identifiants restent neutres ; nom source, ownership, étendue et durée de vie restent UNKNOWN.

Le détecteur de vtable exige une initialisation explicite du champ zéro avec une adresse de table, plusieurs entrées exécutables connues et une chaîne jusqu'à un appel indirect. Il produit alors objets, vtables, groupements de classe et VirtualCallCandidate. Deux loads suivis de bctrl ne suffisent pas. Constructeurs, destructeurs et héritage : NOT_IMPLEMENTED.

## Contrôle de preuve machine

`semantic_proof.rs` accepte seulement une valeur unique sans définition inconnue, dans un calcul convergé et un CFG pris en charge. Il contrôle origine, alias/effets d'appel, région exécutable, entrée exacte et transfert inconditionnel avec lien. Les ensembles finis sont conservés mais non promus : l'IR actuel n'a pas de forme d'appel statique à ensemble de cibles gardé. Les sauts indirects sans lien restent bloqués.

La chaîne complète est conservée. L'appel accepté reçoit une cible statique dans l'IR, avec la provenance de l'instruction, sa valeur de lien et le type indirect de son événement de trace. Un test synthétique compare PPC/IR/natif. Aucun fallback interprété n'est ajouté.

La transaction contrôle le hash IR courant, met à jour site/cible/graphe/références, recalcule contrats et résumés, invalide les produits dépendants et marque les builds concernés périmés. L'analyse sémantique est redérivée dans une boucle bornée. Fermeture et éligibilité restent soumises aux contrôles existants ; aucune hypothèse n'autorise une exécution réelle.

## Bornes et portée

4 096 fonctions et appels par fonction ; 262 144 opérations/nœuds ; 524 288 arcs et initialisations ; 128 tours CFG/interprocéduraux/types/agrégats ; huit alternatives et 128 définitions par valeur ; 4 096 emplacements et nœuds par chaîne de preuve ; 65 536 observations de champs propagées ; 16 384 candidats objet/agrégat/vtable ; 16 tours de retour vers l'analyse machine. Produits locaux limités à 16 Mio, interprocéduraux à 64 Mio.

Un dépassement produit une erreur explicite ou un fait UNKNOWN marqué, jamais une preuve tronquée acceptée. Arithmétique non prise en charge, sorties CFG opaques, imports inconnus et relations mémoire non démontrées restent des obstacles. Les frontières connues du CFG ne prouvent pas l'absence de code manquant. Les catégories plateforme reposent sur des identités explicites ; UNKNOWN ne devient pas GAME_LOGIC. M10 reste gelé et aucune classification n'ajoute de service Cafe.

## Effets mémoire paramétriques v4

`semantic_memory.rs` décrit les accès relatifs aux registres entrants et les plages globales exactes. Un registre entrant n'est ni un paramètre ABI prouvé, ni un pointeur hôte. Chaque effet garde fonction, bloc, PC, opération IR, largeur, lecture/écriture, origine de stockage, provenance et témoins de composition. Les registres de base autres que r3–r10 sont également substitués ; r1 reste une origine symbolique, sans supposer une pile réelle.

La complétude désigne la couverture des effets dans le CFG connu, pas la précision de toutes les adresses. Une lecture d'adresse inconnue ne devient pas une écriture. Un résumé complet sans écriture peut conserver le stockage même si une adresse lue reste inconnue. Une écriture inconnue interdit cette conservation. Toute région source opaque, frontière de contrôle non résolue, import sans contrat ou effet manquant rend le résumé incomplet. Une fonction partielle n'est jamais déclarée en lecture seule sur la seule absence de stores visibles.

Les accès du bloc d'entrée sont marqués `MUST_ON_NORMAL_RETURN; FAULTS_EXCLUDED` : cette qualification ne promet ni terminaison ni absence de faute. Les autres accès et effets composés restent MAY. Les sorties exceptionnelles ne sont pas utilisées pour fabriquer une écriture certaine. Les valeurs potentiellement échappées par un store sont conservées et substituées ; leur type source et leur durée de vie restent inconnus.

Les appels directs substituent les origines entrantes du callee par les valeurs présentes au site appelant, avec addition exacte des offsets. Les résumés forment une fermeture de may-effects par union déterministe. Les témoins sont des ensembles dédupliqués de sites, pas une pile d'appels dynamique inventée. Un callee incomplet est conservé comme référence de résumé avec effet inconnu : ses accès restent inspectables dans son résumé propre, sans expansion répétée de chaque effet dans tous ses appelants. Les cycles qui font croître indéfiniment un déplacement deviennent UNKNOWN par élargissement absorbant.

## Alias et conservation au point d'appel

Le moteur distingue MUST_ALIAS, MAY_ALIAS, PROVEN_DISJOINT et UNKNOWN. Même adresse et largeur donnent MUST_ALIAS ; plages exactes non chevauchantes donnent PROVEN_DISJOINT. Les plages de même origine symbolique sont comparées avec offsets modulo 32 bits. Deux registres entrants différents peuvent aliaser ; une adresse absente, une largeur invalide ou un dépassement de l'espace d'adressage ne prouvent rien.

Un appel dont les écritures sont complètement décrites invalide seulement les faits qu'elles peuvent recouvrir. Les valeurs conservées gardent un témoin du site d'appel et de ses effets. Les écritures précises tuent aussi la possibilité de réutiliser une initialisation du chargeur devenue obsolète. Les effets incomplets, imports inconnus et substitutions non résolues conservent un clobber explicite, avec le site responsable. La conservation mémoire ne suppose aucune conservation de registre : les effets de registre restent ceux du résumé machine.

Les bornes supplémentaires sont 256 effets et 128 racines/témoins par effet, 128 tours de composition et 4 096 comparaisons d'alias par point d'appel. Un dépassement invalide la preuve concernée. Les résumés sauvegardés ne sont pas réutilisés comme preuves machine ; la promotion redérive tout depuis l'IR et l'image courants.

## Audit et versions

Le rapport interprocédural ajoute `memory_effects`, `call_effects`, `memory_metrics` et les audits des mots de stockage observés par relocation. L'audit des indirects ajoute la chaîne de dépendances, les appels responsables, leurs effets mémoire, les pertes de provenance et les valeurs observées aux entrées des appels. Ces valeurs ne sont pas présentées comme une signature Cafe. Les appels potentiellement aliasants d'un stockage sont des possibilités statiques si ces appels sont atteints, pas la preuve qu'ils l'ont modifié avant une lecture particulière.

Les compteurs d'alias aux points d'appel portent sur les faits stockés effectivement examinés. Les audits de stockage globaux ont un périmètre distinct. Une variation du nombre de lectures doit être rapportée avec son dénominateur ; les nouveaux chemins IR ne doivent pas passer pour une amélioration artificielle du taux de résolution.

Versions v4 : Semantic 4 ; interprocédural, provenance et contrôle de preuve 2 ; effets mémoire, alias et politique d'effets 1 ; Typed IR 4, backend C++ 4 et résumé machine 3. CFG 4, dataflow 5, Native ABI 2 et Runtime ABI 2 restent inchangés. Les identités incluent aussi image, chargeur, relocations, hashes IR et politique d'entrée. Les versions invalident les anciens résumés et artefacts natifs.

## Branche conditionnelle vers LR

`ConditionalLrBranch` représente le `bclr` conditionnel sans lien, dans le sous-ensemble documenté. `BranchCondition` applique BO/BI et l'éventuel décrément de CTR ; la branche prise utilise LR avec ses deux bits bas effacés et la branche non prise continue au PC suivant. LR n'est pas modifié. Les formes avec lien ou bits réservés non pris en charge restent opaques. PPC, IR et C++ généré sont comparés sur les deux issues et les cas limites de CTR.

Cette représentation ne prouve pas la cible LR. Elle conserve la dépendance au registre dans les exigences de contexte, les résumés et la fermeture. Aucun interpréteur n'entre dans le chemin natif final. Le modèle natif M10 conserve son périmètre explicite d'import ; sa capacité de préfixe n'est pas étendue implicitement par cette nouvelle forme IR.
