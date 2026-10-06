# CafeGuestImage

Le parseur conserve les octets et métadonnées du module. `CafeGuestImage` construit une copie dérivée, applique les relocations locales démontrées et expose des régions `GuestMemory`. Aucun fichier `code/`, `content/` ou `meta/` n'est modifié.

## Placement et provenance

Le placement actuel est `LINKED_SECTION_ADDRESSES` : les adresses `sh_addr` du module constituent l'espace de l'image liée. Il ne s'agit pas d'une capture du placement choisi par Cafe OS. Un chargement avec déplacement, un lien entre modules et les règles complètes de FILEINFO restent non implémentés. Les observations de WUT, wut-tools et des chargeurs publics sont distinguées des règles documentées dans [SOURCES.md](SOURCES.md).

Seules les sections non vides PROGBITS/NOBITS marquées ALLOC deviennent du stockage de programme. Imports, symboles, chaînes et autres métadonnées du chargeur sont énumérés séparément, avec leur identité et la raison de leur exclusion. Une table d'import marquée exécutable ne devient pas une implémentation exécutable.

Chaque région conserve l'identité/index/nom/type/flags de section, le début, la fin exclusive sur 64 bits, l'alignement, la plage du fichier compressé ou brut lorsqu'elle existe, l'initialisation, les permissions, les SHA-256 avant/après relocation et la provenance. Les sections compressées proviennent des octets décompressés par le parseur borné. NOBITS n'a aucune plage de données dans le fichier ; son zéro initial est justifié par le type de section, pas par son nom.

Le modèle donne R au stockage mappé, W uniquement avec SHF_WRITE, X uniquement avec SHF_EXECINSTR. Ces permissions de l'image ne prétendent pas reproduire les tables de pages d'un processus vivant. Le nom `.rodata` n'annule jamais SHF_WRITE. Les trous ne sont ni alloués ni initialisés.

Limites : 256 régions ; 128 Mio par section ; 256 Mio d'image ; deux millions de relocations. Les exécuteurs conservent leur limite de **1 Mio par état**. Une validation extrait les seuls blocs enregistrés depuis l'image, avec leurs permissions et leur provenance. Elle ne remplace pas une grande image par un tampon rempli de zéros.

## Relocations

Les records du parseur, le modèle symbolique d'analyse et l'application dans l'image sont distincts. Le sous-ensemble à addend explicite couvre ADDR32, ADDR16_LO/HI/HA et REL24. Les formules du supplément ELF PowerPC sont appliquées uniquement à une cible locale appartenant à une section mappée. S+A est calculé sans débordement ; une branche exige opcode, plage signée, alignement et cible exécutable valides. Les bits non relocalisés sont conservés. Aucun pointeur hôte n'est introduit.

| État | Signification |
| --- | --- |
| APPLIED_PROVEN_LOCAL | Octets dérivés écrits ; symbole, addend, place, cible, octets initiaux/résultants et justification conservés |
| DEFERRED_IMPORT | Bibliothèque/symbole/site connus ; `ImportBindingRequired`, aucune adresse ni trampoline inventé |
| UNSUPPORTED | Famille/addend implicite ou liaison de symbole indéfini/spécial non prise en charge ; champ conservé |
| AMBIGUOUS | Champs superposés ou identité de symbole/import multiple ; aucun membre du conflit n'est appliqué |
| INVALID | Place, alignement, symbole local, cible ou calcul malformé ; champ conservé et exécution rejetée |

Les dépendances non appliquées bloquent toute extraction qui les recoupe. Les validations refusent en outre une image contenant des records INVALID/AMBIGUOUS. Les liaisons `$UNDEF` ne sont pas assimilées à l'adresse zéro. Aucune exception n'est ajoutée pour obtenir un meilleur résultat sur un jeu.

## Vérification et identité

Le constructeur rejette les chevauchements de régions, bornes/débordements, tailles et alignements incohérents, permissions non prises en charge et entrée non exécutable. Le vérificateur recalcule les empreintes, contrôle les plages/identités et les octets des relocations appliquées. La validité structurelle de l'image ne signifie ni que ses dépendances sont résolues ni qu'un contexte est prêt.

L'identité dépend du SHA-256 du module, du type RPX/RPL, de l'entrée, des métadonnées et empreintes des régions, de tous les résultats/dépendances de relocation et des versions. Aucun horodatage ni chemin absolu de machine n'intervient. CAFE_IMAGE_VERSION, CAFE_LOADER_VERSION, RELOCATION_APPLIER_VERSION et EXECUTION_CONTEXT_VERSION valent 1 ; elles participent aux manifests et clés natifs, aux programmes associés à une image et aux produits SQLite.

La migration **006** ajoute des produits d'analyse et des validations avec domaine explicite. SQLite conserve des métadonnées et empreintes, sans dupliquer les sections du jeu. Les commandes reconstruisent l'image depuis un module local rehashé ; une version périmée invalide les produits correspondants. Les anciens fichiers dérivés restent des artefacts historiques, jamais une preuve de résultat courant.

## Valeurs mémoire et audit

`ModuleModel::with_image` permet un audit déterministe avec des lectures supplémentaires dans les régions immuables, résolues et vérifiées. Les anciennes preuves symboliques restent prioritaires. Une initialisation connue d'un global inscriptible ne devient pas une constante ; la contamination mémoire par un alias ou un appel invalide toujours les lectures. Un zéro de NOBITS n'est utilisable statiquement que dans le domaine immuable démontré. Les preuves d'audit ajoutent l'identité de l'image et ses versions.

`image indirect-audit` réexécute l'analyse bornée sur les instructions/fonctions connues et écrit `reports/cafe-indirect-audit.json`. Il n'écrase pas les sites historiques et ne lance pas une nouvelle découverte spéculative.

```text
image status [module-id]
image regions [module-id]
image relocations [module-id]
image verify [module-id]
image indirect-audit [module-id]
```

Les listes respectent `--limit`/`--offset`. Les rapports JSON restent déterministes. Le chargeur ne crée ni pile initiale, ni thread, ni tas de processus, ni services Cafe.
