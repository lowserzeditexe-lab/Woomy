# Native backend contract

The `woomy-codegen` crate translates structurally verified IR in the supported subset into C++17. It never decodes PPC words to generate behavior. Stable numeric block/value IDs form a dispatcher; source strings, symbol names and function IDs never enter C++ syntax. Generation is deterministic, without timestamps. `CPP_BACKEND_VERSION = 3`, `NATIVE_ABI_VERSION = 2`, `NATIVE_VALIDATION_VERSION = 2`.

## Supported machine behavior

The implemented subset includes 32-bit wrapping arithmetic, bitwise operations, rotate/mask, GPR/LR/CTR transfers, aligned big-endian `lwz`/`stw`, direct branches, direct conditional branches, LR returns and unconditional traps. Comparisons produce explicit CR field nibbles. Selected add/subtract variants update CA, OV and sticky SO; record forms copy the current SO into CR0. CTR decrement occurs even when the CR test fails. Conditional LR/CTR branches and linked conditional calls remain unsupported.

Local LO/HI/HA relocations retain their module, symbol/table/index, kind, addend, place and proven local target in IR. The lifter never substitutes the original encoded immediate for the linked value. Imports retain identity and no numeric address. Synthetic relocation references execute an independently specified logically linked copy; original IR source words remain unchanged. The separate Cafe image now applies proven local fields; this IR operation does not perform general linking.

All integer arithmetic uses unsigned bit patterns; signed comparisons flip the sign bit before unsigned comparison. Overflow is computed explicitly. Rotate zero avoids shifting by 32. Guest addresses are never cast to host pointers. Each memory access checks alignment and bounds before touching a byte vector. The backend now supports selected 8/16/32/64-bit transfers, update/indexed addressing, lmw/stmw and double-FPR bit transfers. Unsupported alignment and guest exception delivery remain explicit limits; see [CALLS.md](CALLS.md).

The CPU state contains 32 GPRs, 32 FPR bit patterns, eight CR nibbles, LR, CTR, XER and FPSCR. FPR/FPSCR storage is preserved and compared; this does not implement floating arithmetic. Memory, ordered write records, source-PC trace and termination are also compared. Limits are 1 MiB guest memory and 65,536 source instruction steps per case. A limit or unsupported memory access is INCONCLUSIVE, never a semantic match.

## Eligibility, compilation and ownership

`codegen <id>` reruns the IR verifier and checks source membership/raw words against SQLite. It emits C++ only if every operation and terminator belongs to the backend whitelist and coverage is LIFTED. Unknown instructions/values/effects, opaque memory, unresolved control, unknown call effects and imported runtime addresses produce NOT_ELIGIBLE with CODEGEN_UNSUPPORTED diagnostics identifying the function, block, source PC and operation. There are no successful fallback stubs.

`build <id>` regenerates source from verified IR and compiles it. It does not compile user edits to an existing generated `.cpp`. `find-msvc-tools` discovers Visual Studio's compiler/linker and environment; no user-specific installation path is hardcoded. MSVC receives separate executable/argument calls, fixed source/object/executable filenames, and no shell. CL/_CL_/LINK/_LINK_ environment options are removed. Compilation uses C++17, `/O2`, `/MT`, `/W4`; linking targets x64. Version output, stdout/stderr, exit codes, elapsed time, source hash and output hash are recorded. Missing tools report TOOLCHAIN_NOT_FOUND; no installer runs.

Compiler outputs are first written in a fresh temporary directory beneath the checked Woomy output directory. Final files use atomic publication and the existing path traversal, reparse point and hardlink checks. Project outputs are `.woomy/generated/cpp/<cache-key>.cpp`, `.json`, `.build.json` and `.exe`. The key includes serialized IR/program hash, backend and native ABI versions, call-model/summary/memory/runtime versions, and (for linked programs) every registered function, import binding, provider and limit. SQLite migration 004 adds `codegen_artifacts` and `native_builds`; migrations 001–003 are unchanged. Status checks reject obsolete identities and modified/missing source or binary files. Reanalysis invalidates records through existing function foreign keys; old files are not counted as current.

Compilation yields COMPILED_NOT_SEMANTICALLY_VALIDATED. `codegen` and `build` never execute; the separate explicit synthetic-domain command below has bounded execution. Single-function game artifacts retain runtime target None. The multi-function library accepts explicitly registered guest programs and selected providers, as described in [CALLS.md](CALLS.md); the separate M10 model-validation path supports two explicit imports; no complete Cafe parameter ABI is supplied. Synthetic validation does not update game functions to NATIVE_VALIDATED.

## Native validation

```powershell
cargo run -p woomy-codegen --example validate_native -- target/validation/native
cargo run -p woomy-codegen --example validate_guest -- target/validation/guest
```

The example creates synthetic C++, builds real x64 executables, sends deterministic initial states through a bounded binary protocol and writes `report.json` with initial states and all three final executions. The little-endian transport encodes fields individually; it never serializes host struct layout. Guest memory retains big-endian semantics. Input/output are bounded and subprocess execution has a timeout. The example output directory is explicitly supplied by the developer; the project CLI uses SafeOutput.

Ten fixtures cover arithmetic edge values, ordered memory writes, multiple blocks, PC materialization, traps, comparisons, taken/not-taken branches, bounded CTR loops, BO options, CR/XER updates, HA/LO and HI/LO reconstruction. Each uses 512 seeded initial states. Native reports distinguish MATCH/MISMATCH/INCONCLUSIVE; generation/build/process failures remain CODEGEN_UNSUPPORTED/COMPILER_ERROR/NATIVE_FAULT rather than successful comparisons. The negative tests deliberately change generated arithmetic, inject invalid C++, return a failing process code, fault guest memory and exhaust the instruction budget. They require the corresponding failure or inconclusive result.

A separate IR test checks all 32 BO values × 32 BI values × four CTR values × two CR bit values (8,192 cases), against an explicit expected decision and final CTR. PPC reference logic, IR interpretation and C++ use separate execution implementations; the reference does not call the lifter.

No test contains Nintendo bytes. Generated game source/binaries remain local under ignored `.woomy` paths and must not be distributed with Woomy.

The guest validator adds complete multi-function state/memory/stack and call-trace comparisons. Its native ABI 2 transport includes bounded region metadata/backing bytes and call events. Result kinds distinguish guest faults, unresolved calls/imports and runtime failures from process failures. Recursive guest execution uses a bounded dispatcher, not the host call stack. Negative tests corrupt a call target and require MISMATCH, reject partial/invalid registrations, and compare explicit stops for memory, permissions, depth/call/trace/step limits and unavailable imports.

## Image/context identity and explicit function validation

The image, loader, relocation-applier and execution-context versions participate in every manifest/cache key. Programs associated with an image also retain its module/hash/image identity; obsolete identities are rejected. This invalidates older generated/build records even though backend 3 and Native ABI 2 remain unchanged.

`codegen` and `build` still do not execute their output. The separate opt-in `validate-function <id> synthetic` compiles a supported register-only program from verified IR, supplies loader-derived executable blocks and explicit bounded register inputs, then runs all three executors. Results are stored as GAME_FUNCTION_SYNTHETIC_DOMAIN, never REAL_GAME_CONTEXT_MATCH. See [CONTEXT.md](CONTEXT.md) for bounds, provenance, report retention and snapshot limitations. Generated game code and reports remain under ignored `.woomy` paths.

## External runtime identity boundary

Runtime ABI is now **2**; C++ backend and native transport ABI remain **3/2**. Existing native artifacts are rebuilt because their runtime identity changed. Programs carrying an external runtime binding or unresolved source barriers are rejected by codegen. Native validation also rejects a bound Machine instead of discarding its process identity. M10 entry experiments additionally use the explicit model-validation generator and identity-bearing transport documented in [NATIVE_BRIDGE.md](NATIVE_BRIDGE.md). The normal guards remain unchanged. Snapshot replay can compile a fresh native executable for eligible unbound programs, retaining full-state differential comparison.

## Direction du produit natif

Le produit cible exécute du code hôte reconstruit, sans PPC dynamique. Le [bridge M10](NATIVE_BRIDGE.md) consomme maintenant son ABI C réelle pour le sous-ensemble de validation. Les gardes actuelles contre une identité runtime abandonnée restent intactes. Semantic IR v2 ne pilote pas encore la génération native et ne change aucune éligibilité.
