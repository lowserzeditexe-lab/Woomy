# Woomy IR version 3

Woomy IR is a block-structured machine representation, not reconstructed high-level source. STRUCTURALLY_VALID describes structure, types, CFG and provenance; it never establishes execution equivalence. The structural verifier always leaves executable_semantics_validated false. A separate synthetic differential report describes agreement only for its tested inputs.

## Identity and state

Functions retain their stable function/module IDs, module SHA-256, entry address and IR/decoder/CFG/dataflow versions. Block IDs are PPC start addresses. Operation/result IDs are deterministic (PPC address, slot) pairs. Temporaries are local to a block and defined before use; register reads and writes explicitly carry state across blocks. This is not SSA reconstruction.

Types are I8/I16/I32/I64/F32/F64/Bool/Ptr/Unknown/Opaque. GPRs, LR, CTR, XER, FPSCR and CR fields use 32-bit bit patterns. A CR field contains LT/GT/EQ/SO in its low nibble; bit indices use PowerPC ordering. FPRs use F64 bit storage and reservations remain opaque. A type declaration alone does not implement execution. Address32 represents a guest address, never a host pointer.

## Implemented lifting and execution

- addi/addis, addic/addic., subfic; add/addc/subf/subfc with implemented carry, overflow and record variants.
- ori/oris/xori/xoris, or/and/xor, rlwinm, including their implemented record forms.
- Signed/unsigned immediate and register comparisons; explicit CR LT/GT/EQ and current XER SO.
- LR/CTR transfers and the bl .+4 PC-materialization pattern.
- Byte/halfword/word loads and stores (zero extension and low-bit stores), selected update/indexed forms, lmw/stmw, and lfd/lfdu/stfd/stfdu bit transfers. Effective addresses wrap at 32 bits; accesses check guest bounds/permissions and the supported alignment domain. I32 memory operations may have width 1, 2 or 4; F64 transfers have width 8.
- mulli/mullw (without OE), mulhw/mulhwu, slw/srw and neg (without OE), including supported record forms. High products and six-bit shift counts are explicit.
- Direct jumps and conditional branches. BranchCondition explicitly names the CR bit/expected value, CTR decrement and zero/nonzero test. CTR is decremented before testing either condition, even if the CR test fails. Branch uses a typed boolean and explicit taken/not-taken destinations.
- LR returns with low two bits cleared, and unconditional traps.

IntegerFlags32 selectively replaces CA and/or OV, sets SO on overflow without clearing an existing SO, and preserves unaffected XER bits. Record forms use the updated XER. Carry-in variants, CR logical operations, conditional LR/CTR branches, floating arithmetic and reservations remain unsupported.

RelocationImmediate retains module, symbol/table/index, relocation kind, addend, place, symbolic target and optional proven local guest address. LO/HI/HA values are computed from that target, then interpreted with the instruction's signedness and shift. Raw source immediates remain provenance, never a substitute for linking. Imports retain their identity without a fabricated address and prevent full lifting/native eligibility. Other relocation families and full Cafe linking remain partial.

Calls retain direct/imported/indirect identity and link value with unknown high-level signatures. Separate [call contracts and summaries](CALLS.md) record proven and unknown effects. Individual game IR remains PARTIALLY_LIFTED at unknown call effects. Explicit linked programs can execute calls only through a checked registry/provider; no host parameter ABI or successful call stub is inferred.

## Verification and persistence

Verification checks unique/aligned block and operation IDs, entry and targets, contiguous source coverage, register indices, defined-before-use values, operand/result types, memory widths/endian/fault flags, call metadata/effects, relocation identity shape, branch booleans/CR indices, terminator/CFG agreement and provenance. Unknown source words must remain represented and cannot support a LIFTED claim. Analysis targets remain evidence rather than an execution fallback.

Migration 003 stores IR and verification reports. Migration 004 adds native artifacts/builds without changing earlier migrations. Migration 005 adds call contracts, dependency-checked summaries and runtime import records. IR version 3 invalidates older module caches; dataflow version 5 adds controlled-stack facts. Each function is limited to 4096 blocks, 262,144 operations and 16 MiB serialized JSON; exceeded budgets remain explicit failures.

Commands:

- `ir <id>`: inspection paginated by blocks. Verification covers the full stored function; a page is not a standalone executable artifact.
- `verify-ir <id>`: structure plus source membership/raw-word checks against SQLite.
- `ir coverage`: deterministic counts from stored operations and terminators, one primary blocker per function/block/source PC. Shared instructions can count in multiple functions. The opcode histogram describes unknown instruction operations; symbolic relocation operations have their own category.
- `codegen [status] <id>` and `build [status] <id>`: see [NATIVE.md](NATIVE.md). Compilation does not validate game semantics.

## Differential validation

Independent word-based PPC evaluation and IR interpretation compare the full machine state, ordered writes, source-PC trace and exit. Both are bounded to 65,536 source steps and 1 MiB memory. FPR bit storage/transfers and FPSCR are compared; floating arithmetic is not executed. Returning through LR into known code follows a block or stops explicitly if a block boundary is unavailable.

MATCH requires all executions to complete and agree. Equal incomplete executions produce INCONCLUSIVE. In multi-function comparisons, disagreement on faults or limits is MISMATCH. Invalid IR is rejected. Deliberately corrupted IR and native C++ must produce MISMATCH despite structurally valid source.

```powershell
cargo run -p woomy-ir --example validate -- target/validation/ir-differential.json
cargo run -p woomy-codegen --example validate_native -- target/validation/native
cargo run -p woomy-codegen --example validate_guest -- target/validation/guest
```

The first command runs the original 512-case synthetic PPC/IR report. The native command builds and executes ten synthetic fixtures with 512 deterministic initial states each, comparing PPC, IR and x64 results. Relocation fixtures use independently specified logically linked reference words while retaining unpatched IR provenance. No game function is executed or marked semantically validated.

## Loader-backed context analysis

IR remains version 3. The separate [Cafe image](LOADER.md) applies proven local relocations to derived bytes. [Context analysis](CONTEXT.md) retains read-before-definition registers, symbolic memory requirements, source PCs and call/relocation evidence. Synthetic-domain validation compares the same real function in PPC, IR and native form, without promoting its inferred boundaries or an arbitrary input state to a real game context. No unsupported opcode was lowered in this milestone.
