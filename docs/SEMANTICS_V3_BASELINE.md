# Analyse sémantique v3

Typed IR 3 reste autoritaire. Types, signatures, agrégats et classes sont des hypothèses du domaine **STATIC_MACHINE_IR_HYPOTHESES**. La baseline est conservée dans [SEMANTICS_V2_BASELINE.md](SEMANTICS_V2_BASELINE.md) et [STATUS_M10_BASELINE.md](STATUS_M10_BASELINE.md). Les mesures courantes figurent dans [STATUS.md](STATUS.md).

## Produits et identités

`semantic` redérive les produits locaux et interprocéduraux de toutes les fonctions courantes. `semantic <function-id>` conserve la vue locale détaillée. La vue interprocédurale complète se trouve dans les rapports globaux :

- `reports/semantic-summary.json` : métriques locales historiques, `v3_metrics` et promotions persistées.
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

Les champs des callees peuvent être instanciés à une origine exactement identifiée chez l'appelant. La propagation d'identité prise en charge exige un passage sans décalage de base. Des offsets similaires ne fusionnent pas des origines distinctes. Les identifiants restent neutres ; nom source, ownership, étendue et durée de vie restent UNKNOWN.

Le détecteur de vtable exige une initialisation explicite du champ zéro avec une adresse de table, plusieurs entrées exécutables connues et une chaîne jusqu'à un appel indirect. Il produit alors objets, vtables, groupements de classe et VirtualCallCandidate. Deux loads suivis de bctrl ne suffisent pas. Constructeurs, destructeurs et héritage : NOT_IMPLEMENTED.

## Contrôle de preuve machine

`semantic_proof.rs` accepte seulement une valeur unique sans définition inconnue, dans un calcul convergé et un CFG pris en charge. Il contrôle origine, alias/effets d'appel, région exécutable, entrée exacte et transfert inconditionnel avec lien. Les ensembles finis sont conservés mais non promus : l'IR actuel n'a pas de forme d'appel statique à ensemble de cibles gardé. Les sauts indirects sans lien restent bloqués.

La chaîne complète est conservée. L'appel accepté reçoit une cible statique dans l'IR, avec la provenance de l'instruction, sa valeur de lien et le type indirect de son événement de trace. Un test synthétique compare PPC/IR/natif. Aucun fallback interprété n'est ajouté.

La transaction contrôle le hash IR courant, met à jour site/cible/graphe/références, recalcule contrats et résumés, invalide les produits dépendants et marque les builds concernés périmés. L'analyse sémantique est redérivée dans une boucle bornée. Fermeture et éligibilité restent soumises aux contrôles existants ; aucune hypothèse n'autorise une exécution réelle.

## Bornes et portée

4 096 fonctions et appels par fonction ; 262 144 opérations/nœuds ; 524 288 arcs et initialisations ; 128 tours CFG/interprocéduraux/types/agrégats ; huit alternatives et 128 définitions par valeur ; 4 096 emplacements et nœuds par chaîne de preuve ; 65 536 observations de champs propagées ; 16 384 candidats objet/agrégat/vtable ; 16 tours de retour vers l'analyse machine. Produits locaux limités à 16 Mio, interprocéduraux à 64 Mio.

Un dépassement produit une erreur explicite ou un fait UNKNOWN marqué, jamais une preuve tronquée acceptée. Arithmétique non prise en charge, sorties CFG opaques, imports inconnus et relations mémoire non démontrées restent des obstacles. Les frontières connues du CFG ne prouvent pas l'absence de code manquant. Les catégories plateforme reposent sur des identités explicites ; UNKNOWN ne devient pas GAME_LOGIC. M10 reste gelé et aucune classification n'ajoute de service Cafe.
