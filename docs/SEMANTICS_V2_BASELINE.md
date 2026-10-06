# Analyse sémantique v2

La couche consomme Typed IR 3 vérifié, sans changer le lifter, les instructions, les relocations, les cibles machine ou l'éligibilité native. Domaine : **STATIC_MACHINE_IR_HYPOTHESES**. Les chiffres courants sont dans [STATUS.md](STATUS.md), la baseline v1 dans [STATUS_REALIGNMENT_BASELINE.md](STATUS_REALIGNMENT_BASELINE.md).

## Produits et identités

`semantic` analyse toutes les fonctions courantes ; `semantic <function-id>` retourne une fonction détaillée. Les produits `SEMANTIC` version **2** portent fonction/module/SHA module/hash IR, preuves et métriques. Ils sont redérivés à chaque commande et persistés dans `analysis_products` (SQLite 6). Une ancienne hypothèse n'est jamais relue comme un fait.

La commande globale ajoute `SEMANTIC_TABLES` par module, lié au module et au hash de CafeGuestImage, ainsi que `.woomy/reports/semantic-tables.json`. Les adresses identiques de modules distincts ne sont pas fusionnées. Le résumé global n'est écrit qu'après succès ; une erreur peut laisser des produits par fonction déjà enregistrés.

## Propagation et preuves

Le CFG connu est parcouru jusqu'au point fixe. Domaine de valeurs : constante, base symbolique d'entrée + offset, ensemble conflictuel borné, inconnu. Les registres initiaux désignent des valeurs entrantes symboliques, **pas des valeurs runtime connues**. Les blocs inaccessibles ne reçoivent pas arbitrairement ces entrées.

À une jointure, tous les prédécesseurs accessibles sont considérés. Une valeur compatible identique survit ; des valeurs différentes restent un conflit explicite (au plus quatre alternatives). Un prédécesseur inconnu impose UNKNOWN. Les cycles sont réévalués, avec borne de 256 itérations ; un dépassement échoue explicitement. Chaque valeur garde au plus 256 ancres de définition ; un dépassement devient inconnu. Les faits exposés sont ceux effectivement lus avant définition locale, avec bloc consommateur, opération IR/PC, prédécesseurs, définitions et règle. Les preuves des candidats renvoient à ces faits et au graphe IR identifié par hash.

Copies, constantes, Address32 et addition d'offset signée bornée sont propagées. Les appels et effets opaques tuent tous les faits de registre et valeurs temporaires. Le décrément implicite CTR d'une condition de branche invalide CTR. Aucun contenu mémoire n'est mis en cache : une valeur chargée reste inconnue, et un store/alias incertain ne peut autoriser la réutilisation d'un ancien load. Aucune propagation inter-fonctions ni preuve d'ownership/heap n'est annoncée.

Cette discipline supprime les bases artificiellement recréées à l'entrée de chaque bloc par v1. Une baisse du nombre de candidats représente donc une restriction des hypothèses, pas une perte de couverture des instructions IR.

## Types, signatures et agrégats

Le schéma représente Unknown, Integer, SignedInteger, UnsignedInteger, Float, Double, GuestPointer, FunctionPointer, Array, StructPointer, ClassPointer, Enum et Opaque. Chaque rapport sépare `representable_type_kinds` et `inferred_type_kinds`. Les règles de registre infèrent seulement usages GuestPointer et comparaisons signées/non signées. La largeur 32 bits ne prouve jamais un pointeur.

Un accès mémoire base+offset produit une observation de type et éventuellement LoadField/StoreField ; deux offsets distincts sont requis pour un agrégat. Les accès r1 restent de la pile. Chevauchements et interprétations contradictoires restent Conflicting ; aucun nom métier, extent complet ou layout source n'est inventé.

Les signatures conservent les lectures de valeurs entrantes à travers le CFG. Ces observations incluent l'état de contrôle et la pile. Les usages typés des registres r3–r10 sont des **candidats d'arguments**, jamais des paramètres Cafe prouvés. Les écritures r3 sont distinguées des candidats de retour atteignant un terminator Return connu sans effet opaque ultérieur dans le bloc. Le type de retour source et l'ABI complète restent UNKNOWN ; les analyses de retour inter-blocs et de cohérence entre sites d'appel ne sont pas implémentées.

## Pointeurs de fonction et tables

La passe de stockage part exclusivement de relocations ADDR32 locales appliquées, compare les quatre octets big endian avec la cible de relocation et demande une adresse de début de fonction connue dans une région exécutable. Aucun entier ressemblant à du code n'amorce un candidat sans cette preuve de relocation.

Les suites de relocations contiguës sont bornées à 256 entrées. Alignement, permissions effectives de stockage, identités des cibles et entrées non résolues sont conservés. Une région writable ne devient jamais immuable d'après son nom. Une suite avec conflit reste REJECTED_OR_CONFLICTING ; une suite d'au moins deux entrées sans conflit est une table HYPOTHESIS, pas une frontière de tableau source prouvée. Les pointeurs sont comptés par emplacement de stockage sans conflit, pas par cible distincte.

Les consommateurs recensés sont uniquement les matérialisations de relocation référençant la plage candidate, avec fonction/bloc/PC/opération. Cela ne prouve pas encore un load indexé ni un flot jusqu'à CTR/LR : les listes de sites d'appel restent vides tant que ce lien manque. FunctionPointerLoad/TableLookup/ArrayAccess/AddressOf ne sont pas émis sans cette démonstration. La passe constitue une première analyse de stockage, pas un moteur complet de pointeurs de fonction.

Aucun détecteur complet d'objet/vtable/classe/appel virtuel n'est livré. Le motif load→load→CTR→bctrl ne suffit pas ; les listes correspondantes restent vides. Constructor/Destructor restent réservés. Les zéros mesurés ne prouvent pas l'absence de telles structures dans le jeu.

## Portée et limites

Les catégories plateforme viennent uniquement d'identités explicites. Les 49 sites non classés de la baseline restent UNKNOWN ; aucune fonction n'est classée GAME_LOGIC par défaut. Catégorie et contrat ABI sont distincts.

Budgets : 4 096 fonctions, 65 536 opérations IR/fonction, 16 Mio/produit, 524 288 entrées de relocation examinées, 16 384 suites candidates. Les preuves machine requises pour promouvoir une cible restent une barrière distincte, actuellement fermée : aucun candidat sémantique n'est injecté dans le graphe d'appels. Confiance numérique null ; evidence ≠ confidence.

Les tests synthétiques couvrent offsets/largeurs, jointures identiques/conflictuelles/inconnues, point fixe, effets opaques, absence de cache mémoire, scalaire non pointeur, relocations/cibles/perms de tables, motif virtuel insuffisant, déterminisme, sérialisation, persistance et IR périmé.
