# Audit des symboles indéfinis Cafe — version 1

Résultat du 2 octobre 2026 : **aucune nouvelle règle de résolution suffisamment justifiée, zéro relocation appliquée en plus**. Les 17 références `$UNDEF` restent UNSUPPORTED/UNKNOWN et l'arrêt d'entrée est conservé. La contribution générique est un audit reproductible des symboles SHN_UNDEF, indépendant de leur nom et du jeu.

```text
image link-audit [module-id]
```

La commande relit le module autorisé et vérifie son hash. Elle enregistre identité exacte des symboles, binding/type/other et record brut borné, sections de symboles et de relocation, index global et index de section, familles/addends, voisins, métadonnées FILEINFO et modules disponibles. Elle utilise SafeOutput et `analysis_products` kind LINK_AUDIT (schéma 6), version 1 dans le produit. Les octets du module et l'image ne sont pas modifiés. Limites : 65536 symboles, 2 millions de relocations examinées, 4096 symboles indéfinis/références rapportés, 16 Mio d'artefact ; dépassement explicite. Ce n'est pas un éditeur de bindings ni une commande d'exécution.

## Observations locales

Domaine **EMPIRICALLY_OBSERVED_IN_LOCAL_DUMP**, module `module.Gambit.30647d865809`, SHA-256 `45ecd1ed81a97f46b1c8ae24820cd47fbc55399b50af09bf1f9050fe803308e5`.

- Un seul module du dump sélectionné : `code/Gambit.rpx`. Aucun RPL fournisseur n'est présent dans ce corpus analysé. Cela ne prouve pas que Cafe n'utilise pas de modules système externes.
- Table 35 `.symtab`, section de noms 36, record **3** : `$UNDEF`, `st_value=0`, `st_size=0`, `st_info=0x11` (**GLOBAL / OBJECT**), `st_other=0`, `st_shndx=0` (**SHN_UNDEF**). Le record nul 0 est également indéfini, mais n'est pas la cible des 17 références.
- Les 17 références sont dans la section 7 `.rela.text`, liée à la table 35 et ciblant la section 2 `.text`. Toutes ont addend explicite **0** et visent le même record 3 : **7 HA, 7 LO, 3 REL24**. Le type OBJECT ne justifie pas de convertir les trois sites REL24 en un service.
- Quatorze champs sont dans le voisinage de l'entrée, trois à `0x02C86EFE..0x02C86F08`. Les HA/LO et les tests/appels voisins suggèrent des hooks optionnels de startup (**WOOMY_INFERRED**), sans rétablir leur nom original ni leur liaison.
- Les voisins référencent notamment `$TEXT`, `$DATA`, `__cpp_exception_init_ptr`, `__cpp_exception_cleanup_ptr`, `__ghsLock` et `__ghsUnlock`. Ces proximités sont des indices, pas une substitution de symbole.

| Index global / record dans .rela.text | Champ guest | Famille |
| --- | --- | --- |
| 280547 | `0x02C6BCF2` | ADDR16_HA |
| 280548 | `0x02C6BCF6` | ADDR16_LO |
| 280549 | `0x02C6BCFE` | ADDR16_HA |
| 280551 | `0x02C6BD06` | ADDR16_HA |
| 280552 | `0x02C6BD0A` | ADDR16_LO |
| 280554 | `0x02C6BD12` | ADDR16_LO |
| 280557 | `0x02C6BD20` | REL24 |
| 280558 | `0x02C6BD26` | ADDR16_HA |
| 280559 | `0x02C6BD2A` | ADDR16_LO |
| 280560 | `0x02C6BD30` | REL24 |
| 280563 | `0x02C6BD62` | ADDR16_HA |
| 280564 | `0x02C6BD66` | ADDR16_LO |
| 280565 | `0x02C6BD8A` | ADDR16_HA |
| 280566 | `0x02C6BD8E` | ADDR16_LO |
| 282327 | `0x02C86EFE` | ADDR16_HA |
| 282328 | `0x02C86F02` | ADDR16_LO |
| 282329 | `0x02C86F08` | REL24 |

Le premier record 280547 est à `0x02C6BCF2`, moitié basse de l'instruction à `0x02C6BCF0`. Les zéros dans le champ original et dans st_value sont des données observées, pas une preuve de valeur runtime.

## FILEINFO et liaison inter-module

La section FILEINFO 39 fait 221 octets, préfixe `0xCAFE0402`. En appliquant seulement le layout public WUT, les mots à 0x24/0x28 valent `0x10256C80/0x10008000` (bases SDA/SDA2), celui à 0x2C vaut `0x20000` (taille de pile déclarée), flags à 0x34 = 2. Ces valeurs n'identifient pas un fournisseur de `$UNDEF` et ne deviennent ni r13/r2 ni la pile live du modèle. Le modèle M9 préexistant reste inchangé.

Le symbole 3 ne pointe pas vers une section `.fimport_`/`.dimport_`. Aucune identité bibliothèque/export ne lui est donc attribuable par la règle d'import existante. Les dépendances importées du module sont enregistrées, mais leur simple présence ne prouve pas qu'un autre RPL définit ce symbole. Le corpus à un seul module est insuffisant pour prouver un binding externe ; aucun nom/export fournisseur n'a été établi. L'absence d'une telle preuve ne démontre pas non plus que `$UNDEF` est inter-module.

Le champ historique sh_info de `.symtab` a une valeur spécifique au producteur ; il n'est pas utilisé ici comme inventaire de fournisseurs. La vérification complète des sémantiques FILEINFO/CRC et du producteur reste hors du linker actuel. Aucune donnée de metadata n'est extrapolée en adresse de liaison.

## Sources publiques et décision

**PUBLIC_IMPLEMENTATION_OBSERVATION** : [Decaf, révision e6c528a](https://github.com/decaf-emu/decaf-emu/blob/e6c528a20a41c34e0f9eb91dd3da40f119db2dee/src/libdecaf/src/cafe/loader/cafe_loader_reloc.cpp), `sFixupOneSymbolTable` ignore les symboles de section zéro ; `sExecReloc` reprend leur valeur pour appliquer les relocations. Sa gestion des références faibles est distincte. Il n'effectue pas de recherche spécifique par le nom `$UNDEF` dans ce chemin.

**PUBLIC_IMPLEMENTATION_OBSERVATION** : [Cemu, révision 32e6628](https://github.com/cemu-project/Cemu/blob/32e6628a6a9e5255fd0f7e734386a364ff9c106e/src/Cafe/OS/RPL/rpl.cpp), la correction des symboles saute aussi l'index de section zéro, puis le chemin de relocation consomme symbolAddress. Cette observation explique pourquoi une implémentation peut finir par utiliser le zéro enregistré ; elle n'authentifie pas une liaison retail ni le contexte réel.

**PUBLIC_IMPLEMENTATION_OBSERVATION** : [WUT tools, révision b22de1a](https://github.com/devkitPro/wut-tools/blob/b22de1a3c0c8950c4ad4b4df170c89c05d3196aa/src/common/elf.h), définitions indépendantes Symbol/Rela/FILEINFO ; [elf2rpl](https://github.com/devkitPro/wut-tools/blob/b22de1a3c0c8950c4ad4b4df170c89c05d3196aa/src/elf2rpl/main.cpp), ajustement de valeurs pour les sections possédant les symboles. Ces déclarations aident à lire les records, sans établir une règle spéciale pour ce record indéfini.

**DOCUMENTED** : les distinctions ELF entre symboles indéfinis, absolus et faibles ainsi que les formules LO/HA/REL24 sont déjà documentées dans [SOURCES.md](SOURCES.md). Un GLOBAL indéfini n'est pas un WEAK facultatif ; le nom `$UNDEF` ne modifie pas ses attributs. Les règles de résolution dynamique Cafe complètes restent à établir.

Les deux chargeurs sont des observations d'implémentations indépendantes, pas deux captures du comportement Cafe. Leur traitement est un élément favorable à l'hypothèse de valeur conservée, **pas une preuve suffisante pour changer la politique de liaison Woomy**. On ne peut pas conclure « absent = zéro » à partir d'un nom, de st_value ou d'une proximité de code. Aucun code tiers n'est incorporé.

Conclusion **UNKNOWN**. Il manque au moins une règle Cafe applicable et justifiée par une documentation légitime du producteur/chargeur, un link map authentique, ou une observation reproductible du chargement de ce record avec ses attributs exacts. Si un binding externe est proposé, il faut en plus identifier le module fournisseur et l'export, son placement et la preuve de correspondance. Une règle vérifiée pourra alors être ajoutée génériquement et testée sur tous ses cas, sans condition sur le nom du jeu.

## Frontière et progression indépendante

La frontière sans debugger reste 13 instructions / 3 blocs / 2 appels M9 avant `$UNDEF` ; avec debugger, 19 / 4 / 2 jusqu'au trap. Aucun patch, saut d'instruction, adresse inventée ni faux service n'est introduit. L'audit persiste les inconnues, puis le développement se poursuit via la [fondation sémantique](SEMANTICS.md) et le [design du bridge natif](NATIVE_BRIDGE.md).
