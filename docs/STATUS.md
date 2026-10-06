# WOOMY — SEMANTIC V4 : EFFETS MÉMOIRE INTERPROCÉDURAUX, ALIAS CONSERVATEUR & BRANCHE CONDITIONNELLE VERS LR

Rapport de reprise du **6 octobre 2026**. Ce document est le statut **courant**. La baseline v3 complète (A–AI) est conservée **intacte** dans [STATUS_V3_BASELINE.md](STATUS_V3_BASELINE.md) ; les baselines M9/M10/réalignement et les documents SEMANTICS_V2/V3 restent inchangés. Les règles et bornes v4 sont décrites dans [SEMANTICS.md](SEMANTICS.md) et [CONTEXT.md](CONTEXT.md).

Taxonomie d'évidence employée : `PROVEN`, `EMPIRICALLY_OBSERVED`, `INFERRED`, `HYPOTHESIS`, `UNKNOWN`, `NOT_IMPLEMENTED`, `INCONCLUSIVE`, `PENDING_LOCAL_VALIDATION`.

## 0. Portée de cette session de reprise

Le moteur Semantic v4 (effets mémoire paramétriques, alias conservateur, intégration aux définitions atteignantes, résolution indirecte sous contrôle de preuve, et branche conditionnelle `bclr` sans lien) était **déjà implémenté et intégré** dans le commit importé `433ff64`. Cette session a : reproduit la baseline, audité l'implémentation v4 existante, corrigé une divergence de baseline multiplateforme (Clippy), ajouté une CI, et rédigé le présent statut courant. **Aucune architecture n'a été remplacée, aucune invariante de preuve affaiblie, aucun octet du jeu touché.** Le corpus réel Splatoon n'est pas disponible dans cet environnement : toute mesure de corpus réel est `PENDING_LOCAL_VALIDATION`.

## A. État du dépôt découvert

`lowserzeditexe-lab/Woomy`, branche `main`, commit unique `433ff64f05f767769a6f17e060dfef3f6c61023b`, arbre propre, aucune modification non validée, aucune autre branche. Workspace Cargo (8 crates : `binary`, `ppc`, `ir`, `codegen`, `manmenmi`, `core`, `ai`, `cli`), `unsafe_code = "forbid"`, 6 migrations SQLite, docs volumineuses, fuzz, scripts PowerShell. Le dump du jeu et `.woomy` sont — correctement — absents et ignorés. Travail effectué sur une branche isolée `emergent/semantic-v4`.

## B. État courant de Semantic v4 — `PROVEN` (domaine synthétique)

Implémenté et intégré :

- `crates/core/src/semantic_memory.rs` : `Alias` (MustAlias/MayAlias/ProvenDisjoint/Unknown), `alias()`, `Effect`, `MemorySummary`, `instantiate`, `summarize` (fermeture de may-effects par union déterministe, élargissement absorbant), `apply` (conservation/clobber au point d'appel). Versions `MEMORY_EFFECT_VERSION=1`, `ALIAS_VERSION=1`, `EFFECT_POLICY_VERSION=1`.
- `crates/core/src/semantic_interprocedural.rs` : `apply` câblé dans le point fixe (effets mémoire composés dans la boucle de convergence ligne ~481–490), `call_effects` conservés, classification indirecte (`UNKNOWN_CALL_EFFECT`, `UNKNOWN_ALIAS`, `UNKNOWN_VALUE`, `FINITE_SET_REQUIRES_MACHINE_DISPATCH_SUPPORT`) et `reaching_status` par site.
- `crates/core/src/semantic_memory_reporting.rs` : audit d'effets par site indirect.
- `crates/ir` : `Terminator::ConditionalLrBranch`, `Operation::BranchCondition` (BO/BI + décrément CTR), câblage `lift.rs` / `verify.rs` / `execution.rs` / `calls.rs` / `runtime.rs` ; `crates/ppc` décode `bclr`/`bclrl`.
- `semantic_proof.rs` : contrôle de preuve machine inchangé dans sa rigueur ; promotion limitée aux appels indirects inconditionnels avec lien ; ensembles finis conservés mais non promus.

## C. Déjà complété avant la reprise (par l'agent précédent)

Tout le moteur v4 ci-dessus, plus 15 tests synthétiques v4 (`crates/core/tests/semantic_v4.rs`), les sections v4 de `SEMANTICS.md` et `CONTEXT.md`, et les bumps de versions. `EMPIRICALLY_OBSERVED` : l'ensemble compile et tous les tests passent.

## D. Travail incomplet hérité

1. **Statut courant non rédigé** : `STATUS.md` était byte-identique à `STATUS_V3_BASELINE.md` — le jalon v4 n'était pas documenté comme statut courant. **Corrigé par ce document.**
2. **Reproductibilité multiplateforme de la baseline** : `cargo clippy --workspace --all-targets -- -D warnings` échouait sous Linux (helpers de test utilisés uniquement sous `#[cfg(windows)]`). **Corrigé.**
3. **Absence de CI** : aucun `.github/workflows`. **Ajouté** (vérification Linux + job natif Windows/MSVC).
4. **Ré-audit du corpus réel** : `PENDING_LOCAL_VALIDATION` (dump absent de l'environnement).

## E. Fichiers modifiés cette session

- `crates/codegen/tests/guest.rs`, `crates/codegen/tests/support/guest.rs`, `crates/codegen/tests/support/mod.rs`, `crates/codegen/tests/native.rs` : gating `#[cfg(windows)]` des imports réellement spécifiques à Windows et `#[allow(dead_code)]` sur les helpers utilisés par les exemples et les tests Windows uniquement. **Aucune assertion de test modifiée, aucun comportement produit changé.**
- `.github/workflows/ci.yml` : nouveau.
- `docs/STATUS.md` : ce rapport (l'ancien contenu v3 reste dans `STATUS_V3_BASELINE.md`).

## F. Algorithmes / sémantique ajoutés cette session

Aucun changement de moteur. Les corrections sont de portée outillage/CI/documentation. Le moteur v4 hérité est conservé tel quel.

## G. Tests ajoutés

Aucun nouveau test nécessaire : les 15 tests v4 couvrent déjà les scénarios du cahier des charges (alias must/may/disjoint/unknown, conservation lecture-seule et disjointe, clobber par chevauchement/may-alias/callee incomplet, bases décalées composées sur plusieurs appels, récursion/SCC convergente, valeurs finies non promues, offsets d'agrégats sans fusion d'origines, rejet de politique périmée, `bclr` conditionnel pris/non-pris via CR/CTR, formes avec lien/réservées non supportées, preuves de conservation en exécution native, élargissement sur expansion récursive d'offset, pointeur retourné puis passé, valeurs échappées, entrée non exécutable/entier brut non promus). Ajouter un test redondant aurait été du sur-emballage.

## H. Commandes réellement exécutées (environnement Linux, rustc stable)

```
cargo fmt --all -- --check                               # PROVEN: OK
cargo clippy --workspace --all-targets -- -D warnings    # PROVEN: OK (après correctif multiplateforme)
cargo test --workspace                                   # PROVEN: 170 passed / 0 failed / 3 ignored
cargo build --release                                    # PROVEN: OK (3 m 56 s)
```

## I. Résultats de test

**170 réussis, 0 échec, 3 ignorés.** Les 3 ignorés requièrent des ressources externes absentes : `crates/manmenmi/tests/external.rs` et `crates/codegen/tests/m10.rs` (runtime/source M10 réels via `WOOMY_MANMENMI_*`), et `crates/ai/tests/context.rs` (accès manuel OpenAI). Ils restent `PENDING_LOCAL_VALIDATION`.

## J. Validations indisponibles dans cet environnement

- Compilation/exécution native différentielle (chemins `#[cfg(windows)]`, MSVC requis) : `PENDING_LOCAL_VALIDATION` (job CI Windows fourni).
- Tests externes M10 : `PENDING_LOCAL_VALIDATION` (runtime M10 non fourni).
- Analyse du corpus réel Splatoon (`semantic`, `closure`, audit des 41 sites, intégrité du dump) : `PENDING_LOCAL_VALIDATION` (dump absent, jamais téléchargé).

## K. Changements de sémantique machine

Aucun cette session. (Le moteur v4 hérité introduit `ConditionalLrBranch`/`BranchCondition` ; voir [SEMANTICS.md](SEMANTICS.md) §« Branche conditionnelle vers LR ».)

## L. Changements d'inférence sémantique seule

Aucun cette session.

## M. Impact sur le contrôle de preuve

Aucun. Le portail `MACHINE_PROOF` reste inchangé et non affaibli : valeur unique, provenance complète, définitions compatibles, alias/effets d'appel maîtrisés, région exécutable, entrée exacte connue, transfert supporté, convergence. Les ensembles finis restent conservés **sans** promotion. `UNKNOWN_RUNTIME_ENTRY` est préservé.

## N. Classification avant/après des blocages indirects

Corpus réel : `PENDING_LOCAL_VALIDATION`. Distribution historique v3 conservée et non modifiée : **27 UNKNOWN_CALL_EFFECT, 1 UNKNOWN_ALIAS, 9 UNKNOWN_VALUE, 4 UNSUPPORTED_PATTERN** (41 sites, table complète dans [STATUS_V3_BASELINE.md](STATUS_V3_BASELINE.md) §S). Le comportement v4 pour chacune de ces catégories est `PROVEN` dans le domaine synthétique ; sa mesure sur le corpus réel doit être produite localement avec les commandes de §Y.

## O. Cibles machine réellement prouvées

**0**, inchangé. Seules des fixtures synthétiques positives/négatives établissent le portail de preuve. Aucune cible du jeu n'est assignée.

## P. Graphe d'appels

Inchangé (voir [STATUS_V3_BASELINE.md](STATUS_V3_BASELINE.md) §U : 459 sites, 353 internes, 70 imports, 36 indirects).

## Q. Reconstruction sémantique

Inchangée cette session. Hypothèses (types, agrégats, objets, vtables, classes) restent dans `STATIC_MACHINE_IR_HYPOTHESES`.

## R. Objets / tables / vtables / classes

Inchangé. Constructeurs/destructeurs/héritage : `NOT_IMPLEMENTED` (statut explicite, non remplacé par un compteur zéro).

## S. Blocages imports / plateforme

Inchangés. Deux contrats natifs : `coreinit::OSGetCurrentThread`, `coreinit::OSIsDebuggerInitialized`. Aucun service Cafe inventé. Aucun nouvel obstacle plateforme démontré.

## T. État $UNDEF

**17 restent UNKNOWN/UNSUPPORTED.** Aucun patch, aucune valeur devinée, aucun import fabriqué.

## U. État MANMENMI

**Gelé (M10).** Aucune modification. Aucune réimplémentation du runtime Cafe. Aucun `PPC interpreter/JIT/fallback` introduit dans le chemin natif.

## V. Différentiel PPC/IR

`PROVEN` (synthétique) : les tests `semantic_v4`/`ir` comparent états PPC==IR pour `bclr` conditionnel (toutes combinaisons CR/CTR) et pour les preuves de conservation mémoire. `conditional_link_and_reserved_control_forms_remain_unsupported` confirme que les formes avec lien/bits réservés restent `UnknownInstruction`.

## W. Différentiel natif

`PENDING_LOCAL_VALIDATION` sous Linux (chemins natifs `#[cfg(windows)]`). Le générateur C++ produit bien une source sur toutes plateformes (`generated.source.is_some()` vérifié) ; la compilation/exécution MSVC se fait via le job CI Windows ou localement.

## X. Résultats des tests synthétiques

`PROVEN` : 15 tests v4 + l'ensemble du workspace (170/0/3). Déterminisme vérifié (ex. `recursion_*_converge_conservatively` compare l'analyse après inversion de l'ordre des fonctions).

## Y. Validation du corpus réel — `PENDING_LOCAL_VALIDATION`

À exécuter localement par l'utilisateur sur son dump Splatoon légal (lecture seule), depuis la racine du projet contenant `woomy.exe` et `.woomy/` :

```
woomy semantic                 # recalcule tous les produits ; émet reports/semantic-summary.json,
                               # semantic-interprocedural.json (dont memory_effects, call_effects,
                               # memory_metrics), semantic-indirect-audit.json
woomy closure <entry-id>       # fermeture, imports requis, blocages
woomy calls <function-id>      # graphe d'appels par fonction
```

Comparer ensuite la section N de `semantic-indirect-audit.json` aux catégories v3 (27/1/9/4) et reporter chaque reclassement avec sa raison exacte. **Ne pas** compter un candidat sémantique comme cible machine prouvée.

## Z. Résultats des tests workspace

`PROVEN` : 170 réussis, 0 échec, 3 ignorés (ci-dessus).

## AA. fmt / clippy / release

`PROVEN` : `cargo fmt --all -- --check` OK ; `cargo clippy --workspace --all-targets -- -D warnings` OK (après correctif multiplateforme — auparavant échec Linux dû aux helpers Windows-only) ; `cargo build --release` OK.

## AB. Intégrité SQLite

`PENDING_LOCAL_VALIDATION` (nécessite une base de projet issue du dump). Schéma attendu : **6**. Les tests de pipeline synthétiques passent.

## AC. Intégrité du dump

`PENDING_LOCAL_VALIDATION` (dump absent). Référence historique : 615 fichiers, 1 761 602 316 octets, zéro modifié (voir baseline v3 §AC).

## AD. Intégrité MANMENMI

`PENDING_LOCAL_VALIDATION` (runtime M10 externe absent). Référence historique : 136 fichiers vérifiés (baseline v3 §AD).

## AE. Versions

Inchangées par rapport au moteur v4 hérité, documentées dans [SEMANTICS.md](SEMANTICS.md) : Semantic **4** ; interprocédural / provenance / contrôle de preuve **2** ; effets mémoire / alias / politique d'effets **1** ; Typed IR **4**, backend C++ **4**, résumé machine **3** ; CFG **4**, dataflow **5**, Native ABI **2**, Runtime ABI **2** inchangés. **Aucune version bumpée cette session** (aucun changement sémantique).

## AF. Invalidations de cache

Aucune cette session. Les règles d'identité v4 (module/image/hashes IR/versions/politique d'entrée) restent en vigueur ; un produit sauvegardé n'est jamais consommé comme vérité machine.

## AG. Hashes / identités

Identité interprocédurale v3 de référence : `6c38707d7143c8dde42683947c27f4c7d5111d0bb76226717e716759673d6572`. Module SHA-256 : `45ecd1ed81a97f46b1c8ae24820cd47fbc55399b50af09bf1f9050fe803308e5`. Image : `77fe959643e3db31964dd743fce074884cdb373b0216edd9b9063076cbb5d273`. Les hashes d'exécutables/natifs sont `PENDING_LOCAL_VALIDATION` (build release de validation produit localement).

## AH. Limites restantes

Appelants non découverts, effets des imports, adresses runtime, alias non résolus, stockage mutable à l'entrée, 17 $UNDEF, dispatch à ensemble fini, promotion des sauts sans lien, sous-objets décalés génériques complets, durée de vie des objets, constructeurs/destructeurs/héritage — tous `UNKNOWN`/`NOT_IMPLEMENTED`. La cible LR du `bclr` conditionnel reste `UNKNOWN` par conception.

## AI. Distribution exacte des blocages

Corpus réel : `PENDING_LOCAL_VALIDATION` (dernière mesure connue 27/1/9/4, §N). Domaine synthétique : comportement de chaque catégorie `PROVEN` par les tests v4.

## AJ. Prochain jalon recommandé

1. **Exécuter §Y localement** sur le dump pour obtenir la distribution réelle v4 des 41 sites et mesurer la réduction éventuelle de `UNKNOWN_CALL_EFFECT`, en reportant chaque reclassement avec preuve.
2. Si les effets d'appel restent le blocage dominant, étendre la précision des résumés sur les callees incomplets **sans** affaiblir le portail de preuve.
3. Étudier une forme Typed IR de dispatch à ensemble fini gardé (les ensembles finis sont déjà conservés mais non promouvables faute de représentation IR saine).
4. Poursuivre multi-objets / bases décalées / tables modifiées côté hypothèses sémantiques.

Invariants maintenus : origine exacte, preuve complète avant promotion, validation différentielle, M10 gelé, `Unknown != 0`. Les captures Cafe peuvent enrichir les preuves sans devenir un prérequis aux améliorations statiques génériques.
