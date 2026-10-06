# Woomy CLI

Woomy is a **Wii U Game Reconstruction & Recompilation Framework**, currently developed as a Windows Rust CLI. Splatoon is its primary real-world analysis and validation corpus. The target is native reconstructed game code using MANMENMI, a native PC compatibility layer. PPC execution is an engineering validation tool; the final product must not require a PPC interpreter or JIT. **Woomy does not yet build or boot a complete native game.**

The official [project direction](docs/DIRECTION.md) preserves three converging axes: machine/linking coverage, semantic reconstruction, and native execution through MANMENMI. Static analysis does not require MANMENMI; explicit bootstrap model experiments use the existing external C++ runtime.

The verified milestone, local analysis counts and remaining work are recorded in [docs/STATUS.md](docs/STATUS.md).

The source workspace and a dump are separate concepts. Launch Woomy in a directory containing `code/`, `content/`, `meta/`, or pass `--project <dump-root>`. It never downloads games or keys, decrypts content, moves dumps, or writes into those three directories.

## Build

Windows x64: Rust stable (tested with 1.98.1), Visual C++ Build Tools with the Windows SDK. No Node, Tauri or WebView.

```powershell
.\scripts\setup.ps1             # Check developer tools
.\scripts\setup.ps1 -Install    # Install missing development tools via winget
cargo build --release
```

If PowerShell blocks local `.ps1` scripts, invoke one with a process-only policy override, for example `powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\test.ps1`. This does not change the machine's execution policy. The Cargo commands can also be run directly.

The result is `target/release/woomy.exe`. SQLite and TLS support are bundled. Copy the executable into the dump root; no companion Woomy service is needed. The standard Windows C runtime must be available.

## First launch

```powershell
cd D:\WoomySplatoon
.\woomy.exe
```

Woomy detects the three folders, reads local `meta/meta.xml`, asks to create the project, optionally configures AI, creates SQLite and scans the dump. Unknown metadata is displayed honestly; region is the raw metadata mask. A missing dump produces an error and no project.

Generated files live under `.woomy/`: `project.toml`, `woomy.db`, `cache/`, `generated/`, `adaptations/`, `logs/`, `checkpoints/`, `reports/`. Checkpoints currently live in SQLite. The config stores provider/model/budgets, never keys.

For the existing layout in this workspace:

```powershell
.\target\release\woomy.exe --project '.\Splatoon\Games\Splatoon' status
.\target\release\woomy.exe --project '.\Splatoon\Games\Splatoon' analyze
```

The separate update dump can be selected explicitly. Woomy does not merge updates or move either dump.

## Commands

Launch without a command for `woomy>`. Every listed command has `help <command>`.

| Command | Behavior |
| --- | --- |
| `setup --yes --no-ai` | Noninteractive creation and initial scan |
| `status` | Database, counts, current capabilities and metadata change check |
| `scan` | Stream SHA-256 for every file under the three dump directories |
| `analyze`, `resume` | Scan, parse, discover/decode reachable PPC, CFG, calls, xrefs, liveness and bounded value/control-flow fixed points |
| `modules`, `module <id>` | Module headers and sections |
| `functions [search]`, `function <id>` | Stable IDs, inferred entries and their evidence |
| `disasm <function-id>` | Structured operands, semantics, raw words and status |
| `cfg <function-id>`, `dataflow <function-id>` | Blocks/edges and conservative register liveness |
| `values <function-id>`, `definitions <function-id>` | Inferred register values and reaching definitions |
| `dataflow_status <function-id>` | Convergence, analysis version and work count |
| `indirect [function-id]`, `indirect_targets <function-id>` | CTR/LR resolution, separate return sites, static/imported targets and evidence |
| `jump_tables [function-id]` | Bounded candidates, status, count, default branch and evidence |
| `ir <function-id>` | Typed machine IR, paginated by blocks, with explicit unknown operations |
| `calls <function-id>`, `call <function-id> <hex-site>` | Versioned call identities, provenance and conservative effects |
| `summary <function-id>` | Fixed-point register/memory effects, dependencies and stack observations |
| `runtime status`, `runtime imports` | Reference readiness, model contracts, unresolved imports and call analysis |
| `runtime connect <M10-source-directory>` | Build the adapter against your existing M10 ABI/core/cafe sources |
| `runtime process`, `runtime thread [debugger-absent\|debugger-present]` | Explicit values, provenance, mappings and unknowns |
| `entry frontier`, `entry run-model debugger-absent\|debugger-present` | Static candidate fronts or explicit bounded PPC/IR model execution |
| `snapshot replay <.woomy-relative-file>` | Bounded replay retaining origin; no automatic reference promotion |
| `native status <function-id>` | Compile status and unfulfilled guest execution prerequisites |
| `image status/regions/relocations/verify [module-id]` | Derived linked-address image, permissions, local relocation results and unresolved imports |
| `image indirect-audit [module-id]` | Rerun known indirect-site analysis with proven immutable loader values; preserve historical evidence |
| `semantic [function-id]` | Persist static hypotheses; full-corpus analysis also applies the separate machine proof gate to supported indirect calls |
| `image link-audit [module-id]` | Audit undefined symbols, relocation families/neighbors and metadata; never invent bindings |
| `context <function-id>`, `closure <function-id>` | Input provenance, memory/call prerequisites and known dependency closure |
| `imports catalog` | Unique imported services, known callers/sites and separate model contracts |
| `validate-function <function-id> synthetic` | Explicit bounded execution of eligible register-only real function bytes in PPC/IR/native; never a real game context claim |
| `snapshot inspect/validate <relative-file>` | Bounded Woomy snapshot format and current image identity checks, relative to `.woomy` |
| `verify-ir <function-id>` | Structural/type/CFG/provenance verification; no execution-equivalence claim |
| `ir coverage` | Deterministic blocker histogram derived from stored IR, with per-function counts |
| `codegen <function-id>`, `codegen status <function-id>` | Verify IR, check backend eligibility, emit deterministic C++ and a versioned manifest |
| `build <function-id>`, `build status <function-id>` | Detect MSVC, compile/link eligible IR, persist diagnostics and hashes; no game execution |
| `xref to\|from <hex-address>` | References in all analyzed modules, with module IDs |
| `imports [module-id]`, `exports [module-id]` | Local tables and imported symbols when present |
| `problems`, `unknown`, `runs` | Limitations, unknown opcode counts/examples and run history |
| `config performance eco\|balanced\|maximum` | 100k / 250k / 500k reachable instructions per module |
| `pause`, `cancel`, `resume` | Stop/restart shell analysis at safe checkpoints |
| `clear`, `exit` | Terminal control and graceful exit |

Command mode supports `--json`, `--no-color`, `--limit 1..1000`, `--offset N`. JSON results are on stdout; real progress is on stderr. List output includes `has_more`. Shell pagination uses the limits selected at launch. All addresses passed to `xref` are hexadecimal.

```powershell
woomy status --json
woomy functions --limit 20 --offset 20 --json
woomy function module.synthetic.<path-hash>.00001000
```

Shell analysis runs in one worker within the same process. While it runs, `status`, `pause`, `cancel`, `help`, and `exit` remain responsive. Pause waits for a cancellation check, rolls back the incomplete module and preserves completed modules. Resume rehashes the dump, skips matching completed modules and restarts an incomplete module from its beginning. It does not resume mid-function.

One Woomy process owns a project at a time. A project lock prevents competing analysis processes. SQLite uses WAL, foreign keys, FULL synchronization, a busy timeout and versioned migrations. An interrupted run is detected after reopening; interactive launch offers resume. Ctrl+C requests cancellation; the second press exits immediately, allowing SQLite recovery on the next launch.

## Implemented analysis and limits

- Bounded Cafe ELF32 big endian parser: sections, flags, entry, zlib compression, imports/exports, symbols, REL/RELA records. Max file/total decompressed section bytes: 256 MiB; individual section: 128 MiB; aggregate names: 32 MiB. No unsafe binary casts.
- Partial structured scalar PPC decoder, GPR/FPR/CR/LR/CTR/XER/FPSCR and opaque reservation model, uses/defs and memory effects for supported instructions. Unknown instructions retain raw bytes, unknown semantics and an unknown CFG exit. No linear sweeping past an unknown opcode.
- Function seeds from executable entry, exports, function symbols, direct calls and function-symbol relocations. IDs use module path identity and address. Boundaries are **INFERRED**, even when supporting evidence comes from **REAL** bytes.
- CFG and call graph remain separate. Traversal is limited to 16,384 instructions per function and two million function/instruction memberships per module. Unseen successors and other function entries become unknown CFG exits. Overlapping functions may share instruction membership; no claim of definitive ownership.
- Logical explicit-addend `ADDR32`, `ADDR16_LO/HI/HA` and `REL24` relocation subset, with module/section/offset addresses and symbolic imports. Original instruction words remain unchanged. Matching HA+ADDI or HI+ORI pairs propagate addresses; invalid, overlapping or unsupported records remain unknown. Static REL24 targets are range checked; imported runtime addresses remain unknown. The separate derived Cafe image applies this proven local subset; cross-module linking remains unsupported.
- Deterministic liveness plus inter-block reaching definitions and scalar/symbolic value propagation. Unknown effects and calls invalidate values; no register-preservation ABI is assumed. Joins retain up to four alternatives and 32 reaching definitions before widening. Each function is bounded by 4096 blocks, two million transfer steps and 262,144 recorded uses/writes. Unconverged facts are not published.
- CTR/LR targets feed function discovery, CFG, call graph and XREF through a maximum of 16 discovery rounds. Each persisted resolution carries register values, definitions and an analysis version. Return sites are recorded separately from the non-return indirect count. Proofs depend on explored CFG paths and inferred function boundaries; unresolved exits stay explicit.
- Jump-table proof currently supports unsigned `cmplwi`/`bgt` bounds followed by `rlwinm`, `lwzx`, `mtctr`, `bctr`. At most 4096 demonstrably bounded immutable words are read. Mutable tables remain partial; absent bounds never trigger a scan. Cases preserve duplicate targets and the default branch. Candidates/entries/evidence are stored in SQLite.
- Atomic `lwarx`/`stwcx.` effects and double-word floating loads/stores (`lfd`, `lfdu`, `stfd`, `stfdu`) are decoded. Reservation outcomes remain unknown. Double-FPR bit transfers execute in the validated subset; floating arithmetic remains unsupported.
- Versioned block-structured [Woomy IR](docs/IR.md), a conservative scalar lifter, and a structural verifier. Version 3 retains direct conditional branches with CR/CTR behavior, signed/unsigned comparisons, selected CA/OV/SO and record forms, and local LO/HI/HA relocation immediates with provenance. Unsupported semantics and unknown call effects remain visible. Structural validity does not establish execution equivalence.
- An [IR-only C++ backend](docs/NATIVE.md), Windows x64 MSVC detection, compile/link records and a bounded synthetic native harness. Independent PPC evaluation, IR execution and compiled C++ compare complete state, ordered writes, traces and termination. Migration 004 adds native artifact/build records. `codegen` and `build` generate and compile eligible functions without executing them. Explicit `validate-function <id> synthetic` runs a bounded, separately labeled input domain.
- A [guest runtime](docs/CALLS.md) executes registered synthetic multi-function programs with shared state, permission-checked memory regions, explicit stack, direct/indirect call dispatch, bounded recursion and test-only imports. Migration 005 stores conservative call contracts and fixed-point summaries. Selected byte/halfword/word/double transfers, update/indexed/multiple-register memory forms and multiplication/shifts extend IR/native coverage. Original single-function validation remains available.
- A [derived Cafe image](docs/LOADER.md), conservative [execution context and dependency analysis](docs/CONTEXT.md), bounded snapshot import/validation, and explicit synthetic-domain function validation. Migration 006 stores metadata and domain-specific results; original files remain read-only. Linked section addresses do not establish a live Cafe process.
- Results are keyed by module SHA-256 and explicit parser/decoder/CFG/dataflow/IR versions. Profile changes invalidate analysis. `scan` always hashes; opening/status rechecks module hashes and uses size/mtime for the rest of the dump without claiming a full cryptographic verification.

Not implemented: full Cafe relocation application/cross-module linking; CRC/file-info semantic validation; full Espresso ISA, floating arithmetic/FPSCR execution and paired singles; general memory alias analysis and unconstrained stack recovery; general switch patterns; complete IR lowering and per-game semantic equivalence; complete differential execution beyond the scalar subset; complete Cafe parameter ABI, native external-provider transport and full Manmenmi subsystem integration. `ANALYZED_PARTIAL` means the implemented bounded pipeline ran, **not that a module is fully understood**.

## AI

AI is optional and disabled initially. Setup masks input and stores the key in **Windows Credential Manager**, scoped to the canonical project location. Alternatively set `OPENAI_API_KEY` outside Woomy. No key is accepted on the command line or written to TOML, SQLite or logs. Moving the project may require re-entering a stored key because its credential scope changes.

```text
ai setup
ai provider openai
ai model <your-accessible-model-id>
ai enable
ai test
ai context <function-id>
ai analyze <function-id>
ai explain <function-id>
ai unresolved
ai findings
ai finding <numeric-id>
ai disable
```

Select a model that supports Responses API structured output. `ai test` checks authentication and model access without generation; it does not claim generation compatibility. `ai context` previews the exact bounded payload before transmission. `ai analyze` / `ai explain` send that payload to the chosen provider and can incur API charges. Woomy does not estimate monetary costs. There are no background network requests or automatic retries.

The Context Builder supplies bounded metadata/disassembly/CFG/callers/callees/imports/relocations/liveness, register values/reaching definitions, indirect targets, jump-table evidence and problems. It truncates whole records with explicit flags, never the JSON syntax. It excludes personal absolute paths and full dump files. Names inside binaries remain untrusted data; as with any third-party API, only request analysis of material you intend to transmit.

Responses use a strict schema and local validation. Every accepted finding is **AI_PROPOSED**, never **REAL**. Findings do not change deterministic analysis. Cache keys include exact context (including module hash and analysis versions), provider, model, prompt version and schema version. Network failures leave `WAITING_AI` jobs; repeat the command to retry. Defaults: 32 KiB context, 2048 output tokens, 10 requests per session, one concurrent request, 45-second timeout. Budgets are editable in `project.toml` within validated ranges.

`ai provider mock` explicitly chooses the synthetic offline provider for testing. It does not connect to OpenAI. The read-tool interface validates arguments and allows database inspection only. `search_strings`, remote tool-calling loops, and autonomous `ai run` are **NOT IMPLEMENTED**. `ai stop` reports idle because there is no autonomous scheduler. A single interactive API call is synchronous, bounded by its timeout.

## Safety and testing

Application-generated paths are validated beneath canonical `.woomy/`. Traversal, absolute output paths, alternate streams, hard-linked output files and reparse/symbolic links are refused. Inputs are opened read-only; no dump bytes are copied to output. Atomic config replacement, project locking and SQLite transactions protect normal interruption/crash recovery. This is a local application, not an OS sandbox against another process racing filesystem changes.

Git ignores dumps, databases, keys and credentials, including the existing `Splatoon/` directory. Review staged files before any commit. Nothing is pushed automatically.

```powershell
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --release
cargo run -p woomy-codegen --example validate_native -- target/validation/native
cargo run -p woomy-codegen --example validate_guest -- target/validation/guest
```

Tests generate synthetic binary fixtures. Windows native tests require the installed MSVC C++ toolchain and compile/execute synthetic generated C++; they fail explicitly if the compiler is missing. Normal tests never call a paid API. A Windows Credential Manager test writes and deletes a synthetic test-only secret. Optional manual connection check:

```powershell
# Supply OPENAI_API_KEY securely and choose WOOMY_TEST_MODEL outside this command.
cargo test -p woomy-ai --test context live_openai_connection -- --ignored --exact
```

Fuzz targets for the parser, PPC decoder and CFG are under `fuzz/`; run with `cargo fuzz` on a supported nightly environment (typically Linux/WSL). They are not run by ordinary workspace tests. Technical references and licensing notes: [docs/SOURCES.md](docs/SOURCES.md).

Exit codes: **0** success; **2** configuration/usage or not implemented; **3** missing/invalid/locked project; **4** I/O/database/analysis failure; **5** AI unavailable/rejected; **6** cancelled. Interactive command failures are displayed and the shell remains open.

## MANMENMI M10 integration

See [PROCESS.md](docs/PROCESS.md) for ownership, versions, commands, evidence and capture limitations. The two enabled coreinit services use the actual M10 C ABI and runtime objects. The debugger-absent entry model executes 13 instructions and two calls before the unresolved `$UNDEF` relocation; it remains INCONCLUSIVE. PPC, IR and generated x86-64 now use the same explicit model contracts; no real Cafe capture is claimed.

The current [linking investigation](docs/LINKING.md) leaves `$UNDEF` unresolved. The [semantic foundation](docs/SEMANTICS.md) adds candidates above Typed IR. The [native M10 bridge](docs/NATIVE_BRIDGE.md) is implemented for bounded model validation, with full runtime identity and no PPC/IR interpreter fallback. Semantic v4 adds parametric memory effects, conservative alias checks and selective call clobbers to the v3 argument/return, provenance and reconstruction foundations. A separate machine proof gate remains mandatory. The global `semantic` command writes the interprocedural report and audits every historical indirect site; the function-specific command retains the local detailed view. Native eligibility and execution readiness remain separate, explicit checks.
