# Bridge natif M10 — implémentation et limites

## Implémentation M10

Le contrat consommé est celui de `Manmenmi-M10-NativeRuntime/Manmenmi/abi/include/manmenmi/abi/manmenmi_abi.h`, avec `abi/src/manmenmi_abi.cpp` et les huit unités Cafe/core existantes. Versions C runtime/service/thread/memory : **1/1/1/1**. Transport du provider Woomy : **2**, provider : **2**, protocole spécifique de validation native : **1**. Les contrats historiques Typed IR 4, backend 4, Native ABI 2 et Runtime ABI 2 restent séparés.

Le chemin `entry run-model` prépare un artefact de validation : Typed IR vérifié → C++ généré → compilation x86-64 → `mnm_invoke` → registre de services M10 → continuation native. Aucun interpréteur PPC/IR n'est appelé par ce chemin. Les deux oracles s'exécutent séparément avec deux instances M10 indépendantes. Les fonctions partielles ne deviennent pas des artefacts natifs ordinaires ; les refus de `generate_program` et du transport natif historique restent actifs.

Le sous-graphe accessible est parcouru statiquement. Une barrière source coupe l'émission avant l'instruction concernée et produit le même défaut avant fetch ; aucune valeur ou instruction de remplacement n'est exécutée. Les autres opérations doivent satisfaire le backend existant. Les imports acceptés sont exclusivement `coreinit::OSGetCurrentThread` et `coreinit::OSIsDebuggerInitialized`, sans argument, résultat r3. Une identité bibliothèque/symbole vérifiée au site donne un identifiant de dispatch, jamais une adresse guest inventée.

Une empreinte du programme original, du descripteur, de l'état initial complet, des sources runtime/harness/bridge et des versions est compilée dans le binaire, transmise explicitement et vérifiée/renvoyée par le protocole. Le lanceur vérifie l'état initial et l'empreinte de l'exécutable. La liaison runtime complète reste associée au résultat après vérification de l'identité transportée. Le descripteur inclut module/image/domaine/versions, imports autorisés, processus/thread et hash du helper ; le manifeste du helper vérifie les sources M10 avant et après compilation. Il s'agit d'une chaîne d'identités locale, pas d'une attestation cryptographique d'exécution.

Le `State` natif est l'unique mémoire de l'exécution native. Son transport depuis/vers `Machine` est une initialisation et un résultat de validation ; M10 ne maintient aucune copie de RAM. Les callbacks utilisent ce même State, des adresses uint32, des additions uint64, les permissions et des buffers limités à 65 536 octets. Les écritures sont prévalidées, préallouées et consignées dans l'ordre, octet par octet. Les callbacks sont `noexcept` et convertissent toute exception en erreur.

## Différences avec la proposition historique

- M10 fournit des **pointeurs opaques** `mnm_runtime*`, `mnm_process*`, `mnm_thread*`, pas des handles uint64 avec génération. Ils restent internes au helper natif. Runtime propriétaire, process/thread empruntés ; un propriétaire RAII non copiable détruit le runtime après l'exécution. Aucun handle n'est transporté vers Rust/guest.
- Un seul process par runtime. L'isolation utilise une instance runtime distincte par exécution. Pas de suppression individuelle de thread/process dans l'ABI C ; aucun test de pointeur libéré n'est effectué, car ce serait un accès invalide. Les handles périmés ne sont **pas** validés par l'ABI M10 ; Woomy empêche leur réutilisation par sa durée de vie interne.
- La négociation est `mnm_query_version`, et l'introspection par `mnm_service_count`/`mnm_service_at`. Le dispatch utilise bibliothèque/symbole et `mnm_service_call`, pas la frame complète proposée. Les identités CPU/programme/mémoire restent du ressort Woomy.
- Les structs C utilisent `struct_size`/versions conformément au header, mais aucun bloc de layout hôte n'est sérialisé sur le transport. Les résultats `status` et `has_return` sont contrôlés avant toute écriture de r3. Une erreur ne devient jamais zéro réussi.
- L'implémentation réelle ne vérifie pas `struct_size`/`abi_version` des callbacks dans `mnm_invoke`. Woomy les construit lui-même aux valeurs exactes ; il ne relaie pas de structure callback étrangère.
- `mnm_query_capability` peut allouer sans `catch`. Woomy utilise `mnm_service_at`, protégé par `catch (...)`. `mnm_process_set_current_thread` n'est pas protégé sur son chemin d'erreur allouant ; Woomy ne lui transmet que l'ID rendu par la création réussie du même runtime. Une garantie générale d'absence d'exception pour **toutes** les entrées ABI reste un travail MANMENMI, pas une propriété revendiquée ici.

Les succès restent **MANMENMI_MODEL / SYNTHETIC_VALIDATED**, jamais REFERENCE_VALIDATED. Une concordance à une barrière ou à un trap ne complète pas l'entrée. Les capacités générales de scheduling, reprise, frames ABI arbitraires, captures authentiques et exécution complète restent UNKNOWN/non implémentées.

---

## Proposition antérieure, conservée pour comparaison (DESIGN_ONLY)

# Pont natif Woomy → MANMENMI — proposition ABI v1

**DESIGN_ONLY, non implémenté et non annoncé comme capability.** Runtime ABI Woomy reste 2 ; Native ABI reste 2. La proposition ci-dessous n'est pas l'ABI du helper M9 de validation existant. Aucun code PPC dynamique n'est requis dans le produit cible.

## Appel et ownership

Code C++ généré → shim C stable Woomy → adaptateur C++ MANMENMI → CafeServiceRegistry → implémentation native → code généré. Le shim encapsule les types C++ M9 ; `std::string`, containers, références, vtables C++ et exceptions ne traversent pas l'ABI.

Woomy possède contexte CPU, GuestMemory, bindings d'import, buffers et limites. M9 possède runtime/process/thread/services. Un handle opaque **uint64_t** est un identifiant de registre hôte avec génération, jamais une adresse guest ni un pointeur sérialisé. Woomy crée/détruit les handles explicitement ; leur génération invalide les appels après destruction. Deux processus ne partagent ni contexte ni sélection du thread.

L'adaptateur reçoit une interface mémoire non propriétaire, valable jusqu'à la destruction du runtime et après qu'aucun appel ne soit actif. Toute adresse guest est un **uint32_t**. La mémoire autoritaire reste GuestMemory Woomy ; le bridge ne publie pas son backing et ne convertit jamais l'adresse guest en pointeur hôte.

## Surface C proposée

L'implémentation devra fixer `extern "C"`, convention d'appel Windows x64 et un header contrôlé. Types scalaires uint32_t/uint64_t ; booléens encodés 0/1. Pas de sérialisation de layout hôte. Les structures POD éventuellement utilisées auront size/version en tête, ordre/alignement/offsets documentés et assertions C/C++ ; aucune validation par `sizeof` seul.

Opérations prévues :

- `query_abi(requested_version, out_capabilities)` : versions bridge/runtime/provider/import semantics, fonctionnalités et capacités. Négociation échouée avant création si incompatibilité.
- `create_runtime(memory_callbacks, callback_context_handle, out_runtime_handle)` : enregistre les callbacks ; aucun transfert de propriété de GuestMemory.
- `create_process(runtime_handle, bounded_descriptor, out_process_handle)` : identité image/module/domaine et état explicite ; valeurs absentes conservent UNKNOWN.
- `invoke(process_handle, import_binding_id, bounded_call_frame, out_result)` : liaison prévalidée, arguments fixed-width, identifiant de contexte, version et limites. Le binding provient d'un manifeste vérifié, pas d'une adresse import fabriquée.
- `destroy_process` / `destroy_runtime` : arrêt d'admission, attente bornée ou erreur BUSY, invalidation génération/handles ; callbacks inutilisables ensuite.

Les buffers hôtes d'arguments, résultats, diagnostics et octets mémoire sont empruntés seulement pendant l'appel. Ce sont des buffers API hôtes avec longueur/capacité explicites ; ils ne constituent pas une conversion de pointeur guest. Les diagnostics UTF-8 ont une taille maximale et un indicateur de troncature. Pas de chaînes NUL illimitées.

## Frame, erreurs et mémoire

Le frame contient taille/version, binding d'import, identité runtime/domain/image, GPR/FPR en bits, CR/LR/CTR/XER/FPSCR et budget. La première implémentation peut limiter les imports aux contrats exacts des deux services M9 ; elle doit rejeter les signatures non prises en charge. Aucune convention Cafe générale pour agrégats/variadics n'est déduite de cette représentation.

Le résultat indique status, validité du retour, valeurs et masque explicite des champs modifiés. Les champs non fournis ne sont jamais traités comme zéro. Les validations comparent état complet, mémoire, écritures ordonnées, événements et terminaison comme aujourd'hui.

Codes stables proposés : OK, UNSUPPORTED_ABI, UNKNOWN_IMPORT, UNKNOWN_PROCESS_STATE, INVALID_HANDLE, IDENTITY_MISMATCH, GUEST_MEMORY_FAULT, BUDGET_EXCEEDED, BUSY, INTERNAL_ERROR. Toute exception C++ est capturée dans le shim et convertie en erreur. Aucun unwinding étranger ni succès par défaut.

Callbacks `validate/read/write` : adresse et taille 32 bits, permission explicite, overflow contrôlé en 64 bits, mémoire bornée et adresse entièrement mappée. Les buffers de read/write sont hôtes et bornés, sans pointeur vers le backing guest. Prévalidation complète avant mutation, journal des écritures conservé ; aucune promesse globale de rollback des effets d'un service après erreur sans contrat spécifique. Au départ, invocation synchrone sur un seul appel par processus, sans réentrance ni thread hôte utilisé comme OSThread. Les services async et callbacks game-code demanderont un contrat supplémentaire.

## Conditions avant activation

1. Tests C/C++ indépendants du layout, négociation/versions, handles périmés, isolation processus, mémoire/budgets et aucune exception traversant la frontière.
2. NativeProgramManifest liant code généré, module/image, versions, import set, provider binaire/source hashes et domaine. Invalidations du cache explicitement versionnées.
3. Appels synthétiques depuis du C++ généré utilisant réellement M9, comparaison PPC/IR/natif des deux profils et erreurs ; aucun handler test de substitution.
4. Rejet intact de tout contexte non transportable ; aucun relâchement des gardes actuelles avant passage des tests.
5. RUNTIME_EXECUTABLE pour ce sous-ensemble seulement après exécution native observée. REAL_GAME_CONTEXT_MATCH nécessite toujours une référence légitime.

Le pont est une étape d'exécution native obligatoire ; il ne nécessite pas de résoudre tous les imports ni d'attendre une capture de startup. GX2, AX, input, FS, réseau et scheduler ne sont pas implémentés par cette proposition.
