# Guest calls, memory and runtime

Woomy executes a **synthetic, explicitly registered guest program** through shared machine state. `generate_program`, `execute_ppc_program` and `execute_ir_program` are library APIs. The CLI compiles eligible functions; explicit synthetic validation and Cafe bootstrap commands execute bounded experiments. The derived image loader and external M9 adapter are described in LOADER.md and PROCESS.md. They do not boot a game.

## Evidence and ABI boundary

| Statement | Evidence level | Consequence |
| --- | --- | --- |
| Linked branch updates LR; indirect target uses the old LR/CTR; LR return clears two low bits | Documented common PowerPC instruction behavior | Implemented in the three execution paths |
| WUT RPX startup uses r1-relative stores and saves LR through r0 before calls | Observation of the WUT project's own startup source | Supports recognizing an instruction pattern, not a universal Cafe frame layout |
| WUT OSContext names GPR, CR, LR, CTR, XER, FPR and FPSCR storage | Public implementation evidence | Corroborates state components; does not define parameter passing |
| Cafe argument registers, aggregate/variadic return rules, volatile/nonvolatile sets, required frame size and stack alignment | UNKNOWN for arbitrary game functions | No automatic marshaling or preservation based on a guessed ABI |
| Synthetic arguments/results and TestRuntime imports | Explicit Woomy test contracts | Never asserted to be Cafe APIs |

Primary references and licensing are in [SOURCES.md](SOURCES.md). A generic PowerPC EABI document is not treated as proof of Cafe compatibility. No Nintendo SDK material was used.

## Calls and function summaries

IR `Call` retains its target and unknown signature/effects before linking. A call must end its block with an explicit CIA+4 continuation. Separate versioned contracts identify the caller, site, source provenance, link behavior, internal/imported/indirect/runtime/unresolved target, and KNOWN/PARTIAL/UNKNOWN may-effects. Target identity is separate from effect knowledge.

Function summaries union actual IR register reads/writes, memory accesses, possible traps, return sites, direct callees and unresolved dependencies. A bounded monotonic fixed point propagates effects through cycles. Registers reported preserved are never written or possibly clobbered in that analyzed closure; this is conditional on inferred function/CFG coverage. A register saved and later restored may still appear written. Termination and high-level return signatures remain UNKNOWN. Unknown effects erase preservation claims. No AI participates.

Stack observations describe affine r1 changes and accesses where established from IR. They do not invent source variables. The separate dataflow stack domain records exact aligned word slots relative to symbolic entry r1, intersects facts at joins, and kills slots on unknown aliases/calls. Loading a value from that domain requires `ValueModel::controlled_stack()`. It is false for the real dump: r1 usage alone does not prove a controlled RAM region. Immutable section reads and relocation-derived values retain their existing stricter proofs.

Migration 005 adds function summaries, call contracts and runtime import records. Stored summaries check their own IR hash, a digest of the module's ordered IR dependency set, and contract versions. Reanalysis rebuilds them transactionally. Migrations 001–004 are unchanged. The independent version constants are CALL_MODEL_VERSION, FUNCTION_SUMMARY_VERSION, GUEST_MEMORY_VERSION and RUNTIME_ABI_VERSION; all participate in native artifact identity.

## Dispatch and returns

A `Program` registers stable function identities and guest entries, verified blocks, import site bindings, a provider and limits. Duplicate/overlapping functions, inconsistent module identities/hashes, obsolete versions and excessive program sizes are rejected. Numeric guest addresses are never converted into host function pointers. Native emission reads verified IR, not PPC words.

Internal calls transfer to the registered callee entry. All GPRs, FPR bit patterns, CR fields, LR, CTR, XER, FPSCR and memory remain in the same state object. The dispatcher does not save or restore any of them. Guest stores/loads and SPR transfers implement saves/restores. An auxiliary bounded frame list accounts for depth; it never supplies a hidden return address. The actual LR decides the next guest block. Unregistered return targets stop explicitly. Only the initial outer return target may finish the synthetic harness outside registered code, with no pending frames.

The generator requires every registered implementation to be in its supported subset and every static callee to be registered. A partial callee blocks native generation. An indirect call can only select a registered entry at execution time; unknown targets stop. There is no successful placeholder implementation or interpreter fallback in native output.

## Guest memory and stack

`Machine` owns `GuestMemory` with explicit regions: executable, read-only data, mutable data, stack and synthetic runtime. Each region records a guest base, backing bytes, size (the backing length), permissions and provenance. Accesses check 64-bit range arithmetic, mapping and permissions. Overlap, empty/overflowing regions and excessive backing sizes are rejected. Total region backing is limited to 1 MiB, with at most 256 regions. The legacy flat memory fields remain for the original single-function fixtures; multi-function dispatch requires explicit executable mappings.

The memory API transfers 8/16/32/64-bit big-endian bit patterns. CPU execution covers selected integer byte/halfword/word loads/stores, update/indexed forms, lmw/stmw, and lfd/lfdu/stfd/stfdu bit transfers. Narrow loads zero-extend into I32; narrow stores write the low bits. Update forms capture source/base values before writing the new base. Multiple-register transfers preserve ordered partial completion when a later access fails.

The validated execution domain requires byte alignment for byte transfers, two-byte alignment for halfwords, and four-byte alignment for word/double-FPR transfers. Other alignments stop explicitly. This is a conservative execution restriction, **not a claim that every unaligned Espresso access raises a hardware exception**. MSR/MMU/cache/device behavior and delivery of guest exceptions are not implemented. The guest fault result records the stopped access without pretending to deliver a Cafe exception.

Synthetic cases allocate and zero a bounded stack region and explicitly initialize r1. Tests exercise nested frames and recursion, underflow/overflow, permissions and alignment. No initial game stack or global state is fabricated.

## Providers and imports

`RuntimeProvider` exposes `resolve_import`, `invoke_import` and `query_capability`. `NoRuntime` resolves nothing. `TestRuntime` recognizes only library `woomy_test`:

| Symbol | Explicit TEST_RUNTIME_ONLY behavior |
| --- | --- |
| `test_add` | r3 := r3 + r4 modulo 2^32 |
| `test_return_constant` | r3 := 0x574F4F4D |
| `test_memory_copy` | Copy r5 bytes from guest r4 to guest r3, at most 4096 bytes, with overlap-safe temporary bytes and full source/destination bounds/permission checks |

Other registers, including LR after the dispatcher link update, remain unchanged by these handlers. Copy writes are ordered and compared. Import identity is bound to a call site without fabricating a guest code address; zero in its trace target denotes an import event, not a pointer. Reference effects for real Cafe imports remain UNKNOWN. The two explicit M9 model contracts below are separate from TestRuntime and from reference validation.

Results distinguish return/trap, guest fault, unresolved call/import, runtime failure, invalid IR and resource limit. The source-step budget is supplied per case (maximum 65,536); per-program call count, depth and call-trace limits are configurable and bounded. Different faults, traces or limits are MISMATCH. Equal incomplete executions are INCONCLUSIVE, never MATCH.

## Reproduction and inspection

```powershell
cargo run -p woomy-codegen --example validate_native -- target/validation/native
cargo run -p woomy-codegen --example validate_guest -- target/validation/guest
woomy calls <function-id> --json
woomy call <function-id> <hex-site> --json
woomy summary <function-id> --json
woomy runtime status --json
woomy runtime imports --json
woomy native status <function-id> --json
```

The first validator retains the original 5,120 cases. The second covers direct/nested/multiple/recursive calls, CTR/LR calls, additional memory/integer forms and test imports, with 512 seeded states per fixture. Reports retain initial states and full PPC/IR/native executions including memory writes and call events. They never promote a game function to execution-ready or semantically matched.

## Cafe image and dependency planning

The separate [image loader](LOADER.md) can store up to 256 MiB of derived linked-address sections; execution still permits at most 1 MiB per state. [Context/closure analysis](CONTEXT.md) records missing register and memory provenance, known reachable functions, unresolved indirect exits and actual imported-service identities. `reports/required-imports.json` is a planning queue, not a provider implementation. The explicit M9 model now supplies two coreinit services through the adapter described in [PROCESS.md](PROCESS.md); neither is reference-validated. The explicit synthetic validation command can execute eligible real register-only function bytes, while actual game execution readiness and reference-context equivalence remain separate and unproven.

## M9 model contracts

Runtime ABI 2 binds Program, Machine and external provider to the same process/image/version identity. OSGetCurrentThread and OSIsDebuggerInitialized have separate MODEL_IMPLEMENTED contracts: no arguments, result r3, model-only preservation of other registers and no guest memory effect. Complete Cafe ABI/effects remain UNKNOWN in static summaries. Import catalog and closure preserve reference blockers while exposing model contracts separately. Missing process state is an error, never a success zero.

## Interprétations sémantiques

La [passe sémantique v1](SEMANTICS.md) conserve DirectCall/IndirectCall/PlatformCall avec références aux opérations Typed IR. Une catégorie de plateforme ne fournit ni convention d’appel ni effet prouvé. Aucun VirtualCall n’est résolu par ce jalon et les contrats/summaries machine sont inchangés.

## Effets mémoire v4

Les résumés machine passent à la version 3 avec la dépendance LR de la nouvelle branche conditionnelle Typed IR 4. La couche sémantique calcule séparément des effets mémoire paramétriques, les substitue aux appels directs et compare leurs plages aux faits de stockage. Une lecture d'adresse inconnue n'implique pas une écriture ; une écriture inconnue ou une couverture incomplète empêche la preuve de conservation. Les appels indirects/importés sans contrat restent inconnus. Aucun contrat Cafe n'est ajouté à partir d'une observation de registre. Voir [SEMANTICS.md](SEMANTICS.md) pour les bornes, la provenance, les SCC et les versions.
