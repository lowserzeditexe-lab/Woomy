# Technical sources

Consulted 2026-09-28 through 2026-10-01. Woomy is an independent Rust implementation. No Nintendo SDK, game code, keys, or proprietary fixture bytes are included. Open source implementations below were read for format details; their code was not copied or linked into Woomy. Earlier milestone notes describe the behavior at that milestone; the Cafe image additions are documented below.

- [WiiUBrew RPL](https://wiiubrew.org/wiki/RPL): Cafe ELF overview; import/export tables. Field names denote raw on-disk metadata; no claim of loader equivalence.
- [devkitPro wut-tools elf.h](https://github.com/devkitPro/wut-tools/blob/master/src/common/elf.h): section types/flags, ELF32 structures, symbol and relocation layouts. [readrpl](https://github.com/devkitPro/wut-tools/blob/master/src/readrpl/main.cpp) documents the big endian decompressed-length prefix followed by zlib. Project license: GPL-2.0-or-later. Consulted, not incorporated.
- [Cemu RPL structures](https://github.com/cemu-project/Cemu/blob/main/src/Cafe/OS/RPL/rpl_structs.h) and [header processing](https://github.com/cemu-project/Cemu/blob/main/src/Cafe/OS/RPL/rpl.cpp): independent cross-check of section/header offsets and Cafe identification. Project license: MPL-2.0. Consulted, not incorporated.
- The authorized local RPX has ELF type `0xFE01`; devkitPro's header defines `0xFF01`. Woomy accepts these two variants, retaining strict Cafe ABI, ELF32, big endian, PowerPC and bounds checks. The `0xFE01` observation is empirical evidence, not a claim that Cemu specifies that constant. Synthetic tests cover both.
- [NXP PowerPC programming environments manual](https://www.nxp.com/docs/en/user-guide/MPCFPE_AD_R1.pdf) and [e300 core reference manual](https://www.nxp.com/docs/en/reference-manual/e300coreRM.pdf), instruction tables: standard scalar PowerPC encodings. Woomy implements only the documented common subset; e300-specific behavior is not assumed to describe Espresso. Espresso paired singles and hardware-specific semantics remain unknown.
- [OpenAI Structured Outputs](https://developers.openai.com/api/docs/guides/structured-outputs): Responses API `POST https://api.openai.com/v1/responses`, `text.format` JSON schema, strict output, refusal/incomplete handling and `max_output_tokens`.
- [OpenAI Retrieve Model](https://developers.openai.com/api/reference/resources/models/methods/retrieve): authenticated `GET /v1/models/{model}` connection check. This checks access, not generation capabilities. Model IDs are supplied by the user; Woomy has no hardcoded default.
- [System V PowerPC ABI supplement](https://refspecs.linuxfoundation.org/elf/elfspec_ppc.pdf), relocation table 4-8: `R_PPC_REL24` modifies the branch displacement. A relocation from a relative `bl` to a code import with zero addend identifies an imported call. Woomy records its library/symbol and keeps its runtime target unknown; it does not invent the address or apply a loader patch.

The parser records relocations without applying them. It does not validate Cafe CRC/file-info semantics yet. Unknown section types retain their raw headers and bounded bytes. No calling convention or generated C++ behavior is inferred from these format references. Indirect targets require separate recorded dataflow or bounded-table evidence.


## Advanced analysis references - 2026-09-29

- [PowerPC ELF ABI supplement](https://refspecs.linuxfoundation.org/elf/elfspec_ppc.pdf), table 4-8: explicit-addend ADDR32, LO, HI, HA and REL24 formulas, halfword locations and signed branch range. Woomy models these symbolically; it neither patches the dump nor assigns runtime addresses to imports. S+A may designate another allocated section; the result must map uniquely inside the local module. Unsupported REL/RELA behavior remains unknown.
- [NXP PowerPC programming environments manual](https://www.nxp.com/docs/en/user-guide/MPCFPE_AD_R1.pdf), instruction descriptions lwarx (8-141), stwcx. (8-249), lfd/lfdu (8-118/119), stfd/stfdu (8-230/231): encodings, effective addresses, reservation effects, CR0/XER dependency, 64-bit memory transfer and update restrictions. Woomy records reservation uncertainty and possible faults. No floating arithmetic, execution engine, Espresso paired-single behavior or e300-specific feature is inferred.
- Reaching definitions, finite value alternatives, worklist propagation and switch proofs are original implementations tested with generated fixtures. Register facts and target proofs are INFERRED and conditional on the known CFG. Writable sections are not assumed constant because of their names. No source code or proprietary instruction bytes were copied into tests.

## Native backend references - 2026-09-30

- [NXP PowerPC programming environments manual](https://www.nxp.com/docs/en/user-guide/MPCFPE_AD_R1.pdf), instruction descriptions bc (8-24), cmp/cmpi/cmpl/cmpli (8-30 through 8-33), addic/addic. (8-14/15) and integer add/subtract forms: BO/BI, CTR decrement before the condition test, CR LT/GT/EQ/SO and XER CA/OV/sticky SO. Implemented only for the common 32-bit scalar subset. The manual is copyrighted documentation; no implementation code is incorporated. This does not establish Espresso floating-point or paired-single semantics.
- [PowerPC ELF ABI supplement](https://refspecs.linuxfoundation.org/elf/elfspec_ppc.pdf), table 4-8: LO/HI/HA field values and S+A. IR preserves relocation identity and uses a locally proven target, with signed/unsigned consumption determined by the instruction. External imports have no invented numeric address. Synthetic reference words are independently linked copies for testing, never patched game files.
- [Microsoft compiler command-line syntax](https://learn.microsoft.com/en-us/cpp/build/reference/compiler-command-line-syntax?view=msvc-170) and [MSVC linker reference](https://learn.microsoft.com/en-us/cpp/build/reference/linking?view=msvc-170): separate compilation/linking arguments, exit status and CL/LINK environment behavior. Used to design explicit process invocation and remove implicit option injection. No Microsoft sample implementation was copied.
- `find-msvc-tools` 0.1.14, local crate documentation and public API (`find_tool`, `Tool::to_command`): Visual Studio environment discovery via the maintained Rust toolchain helper. Dependency licensed MIT OR Apache-2.0; used as a library, not copied into Woomy. No absolute developer installation path is embedded in source.
- C++ dispatcher, CPU state, guest memory checks, binary test protocol and three-way comparisons are independent Woomy implementations. MSVC version and equivalence results in STATUS.md are local test observations, not documentation claims.

## Guest runtime references - 2026-10-01

- [WUT RPX startup](https://github.com/devkitPro/wut/blob/master/libraries/wutcrt/crt0_rpx.s) and [OSContext declarations](https://github.com/devkitPro/wut/blob/master/include/coreinit/context.h), devkitPro's public implementation, zlib license: observed r1-relative frame operations, LR save through r0, and named context registers. Consulted, not copied. These observations do not establish the complete Cafe ABI. Argument/aggregate/variadic rules, volatile/nonvolatile sets, universal stack alignment and frame requirements remain UNKNOWN in Woomy. [WUT RPL startup](https://github.com/devkitPro/wut/blob/master/libraries/wutcrt/crt0_rpl.s) references __eabi; that symbol alone is not proof of compatibility with every generic PPC EABI rule.
- [NXP PowerPC programming environments](https://www.nxp.com/docs/en/user-guide/MPCFPE_AD_R1.pdf), chapters 4/8 (linked branches, byte/halfword/word transfers, update/indexed forms, lmw/stmw, multiplication and shifts), plus section 6.4.6.1: instruction-level reference. Update stores consume the old source value; indirect linked branches capture their target before LR changes. Unaligned behavior can depend on processor/memory configuration. Woomy stops outside its validated alignment domain and does not invent Espresso exception delivery. Double-FPR transfers preserve bits; they do not establish floating arithmetic semantics. Independent implementations and synthetic fixtures; no manual code or Nintendo bytes incorporated.
- [CALLS.md](CALLS.md) separates documented architecture, public implementation observations, Woomy synthetic contracts and UNKNOWN Cafe ABI rules. That milestone used no third-party runtime. The subsequent M9 integration below links the user-supplied core/cafe sources through an explicit adapter.

## Cafe image and context references — 2026-10-01

| Evidence class | Source and scope |
| --- | --- |
| DOCUMENTED | [PowerPC ELF supplement](https://refspecs.linuxfoundation.org/elf/elfspec_ppc.pdf), sections/relocations, table 4-8: NOBITS storage, explicit S+A, LO/HI/HA and REL24 formulas. Applied only where Cafe section/symbol identity and the selected linked-address placement are independently established. This is not proof of Cafe process initialization. |
| PUBLIC_IMPLEMENTATION_OBSERVATION | [wut-tools ELF declarations](https://github.com/devkitPro/wut-tools/blob/master/src/common/elf.h) and [readrpl](https://github.com/devkitPro/wut-tools/blob/master/src/readrpl/main.cpp): flags, types, undefined symbol classification, big-endian decompressed-size prefix and zlib. GPL-2.0-or-later; consulted, not copied. |
| PUBLIC_IMPLEMENTATION_OBSERVATION | [WUT RPX startup](https://github.com/devkitPro/wut/blob/master/libraries/wutcrt/crt0_rpx.s): startup consumes an existing guest stack and calls runtime services; it does not provide a captured initial game state. zlib license. |
| PUBLIC_IMPLEMENTATION_OBSERVATION | [wiiu-env EnvironmentLoader ElfUtils](https://github.com/wiiu-env/EnvironmentLoader/blob/main/source/ElfUtils.cpp), [payload_loader](https://github.com/wiiu-env/payload_loader), [librpxloader](https://github.com/wiiu-env/librpxloader): independently consulted loader projects; their explicit homebrew placement/runtime environments are not generic retail Cafe guarantees. No implementation copied or dependency added. |
| PUBLIC_IMPLEMENTATION_OBSERVATION — cross-check only | [Cemu RPL loading](https://github.com/cemu-project/Cemu/blob/main/src/Cafe/OS/RPL/rpl.cpp): zero-filled NOBITS and separate text/data/loader metadata allocation; section-relative placement can differ from original virtual addresses. MPL-2.0; consulted only. Cemu is not the specification and its HLE/trampoline behavior is not reproduced. |
| PUBLIC_IMPLEMENTATION_OBSERVATION — secondary context | [WiiUBrew RPL overview](https://wiiubrew.org/wiki/RPL): Cafe format context. Implementation decisions are checked against the primary declarations/manual above. |
| EMPIRICALLY_OBSERVED_IN_LOCAL_DUMP | The authorized module has six nonempty ALLOC PROGBITS/NOBITS sections; `.rodata` has SHF_WRITE; 17 relocations refer to `$UNDEF`. Local reports preserve hashes/ranges/identities. These observations are not redistributed test bytes or universal format rules. |
| WOOMY_INFERRED | Identity placement at linked `sh_addr`, conservative entry requirements, bounded closure and imported-service work queue. The image hash identifies this model. None establishes a live Cafe stack, global values or caller state. |
| UNKNOWN | Actual game process placement/state, `$UNDEF` binding semantics, complete Cafe ABI, arbitrary mutable memory, import effects and Espresso paired-single behavior. They remain blockers. |

The [NXP PowerPC manual](https://www.nxp.com/docs/en/user-guide/MPCFPE_AD_R1.pdf), dcbst (8-49) and fsub (8-105), identifies the common scalar/cache or floating encodings among three of the four unknown words. Cache coherence/exception behavior and FPU/FPSCR arithmetic are outside the validated execution subset; they were not replaced by no-ops or host arithmetic. The primary-4 word remains opaque without an adequate Espresso-specific proof. All four sites are reachable in the known entry closure and remain UNKNOWN in the decoder.

[LOADER.md](LOADER.md) and [CONTEXT.md](CONTEXT.md) describe the implemented restrictions, evidence, snapshot trust boundary and distinction between synthetic-domain matches and reference-context matches. No proprietary SDK, implementation code or Nintendo bytes were incorporated.

## MANMENMI M9 / process-thread evidence — 2026-10-02

- **PUBLIC_IMPLEMENTATION_OBSERVATION**: [WUT thread.h](https://github.com/devkitPro/wut/blob/master/include/coreinit/thread.h), independent zlib-licensed declarations: OSThread size 0x6A0/alignment 8, context prefix, tag/state/id, stackStart 0x394/stackEnd 0x398/entryPoint 0x39C, zero-argument OSGetCurrentThread pointer return. The default-thread/core-1 RPX-entry note is a public observation, not a captured Splatoon thread or address. No WUT implementation copied.
- **PUBLIC_IMPLEMENTATION_OBSERVATION**: [WUT context.h](https://github.com/devkitPro/wut/blob/master/include/coreinit/context.h), OSContext size 0x320 and tag/register offsets; [debug.h](https://github.com/devkitPro/wut/blob/master/include/coreinit/debug.h), OSIsDebuggerInitialized BOOL declaration; [RPX startup](https://github.com/devkitPro/wut/blob/master/libraries/wutcrt/crt0_rpx.s), debugger branch/trap and use of existing stack. These are not official Cafe specifications.
- **DOCUMENTED, corroboration only**: [PowerPC ELF ABI supplement](https://refspecs.linuxfoundation.org/elf/elfspec_ppc.pdf), printed page 3-22, scalar/pointer result in r3. Does not establish full Cafe call-clobber, aggregate or variadic rules.
- **PUBLIC_IMPLEMENTATION_OBSERVATION, cross-check**: [Cemu thread implementation](https://github.com/cemu-project/Cemu/blob/0a2b6ff7db61871b0dd028ac47dc72eda94351a1/src/Cafe/OS/libs/coreinit/coreinit_Thread.cpp), current-thread association with the executing core. MPL-2.0, consulted only. [WiiUBrew Coreinit.rpl](https://wiiubrew.org/wiki/Coreinit.rpl) is secondary context for debugger semantics, not proof of a particular game's state. Neither is an official specification or runtime dependency.
- **WOOMY_INFERRED / local implementation evidence**: user-supplied MANMENMI M9 `docs/integration/WOOMY.md`, `M9_EXPORT.md`, `cafe/include` and `cafe/src`. Actual runtime/process/thread/service ABI 1; six services advertised AVAILABLE/MODEL_IMPLEMENTED. The adapter compiles eight source units and records 22 source/header hashes in `.woomy/manmenmi.json`. No upstream file is changed or vendored. These implementations are models, not reference captures.
- **EMPIRICALLY_OBSERVED_IN_LOCAL_DUMP**: entry calls at 0x02C6BCC4/0x02C6BCCC, r3 copied into r29 then tested for the second result; `$UNDEF` field 0x02C6BCF2 and trap 0x02C6BCEC. Local reports retain exact source/relocation identity; no Nintendo bytes become repository fixtures.
- **SYNTHETIC_VALIDATED**: big-endian materialization, address/permission bounds, process/provider identity rejection, two actual M9 processes, debugger branches and transport memory operations. **UNKNOWN**: real game initial process/thread address, live globals, complete Cafe object/ABI semantics, `$UNDEF` binding and external reference effects. **REFERENCE_VALIDATED**: zero handlers and zero game contexts.

[PROCESS.md](PROCESS.md) distinguishes layout evidence, explicit model choices, actual M9 invocation and future capture requirements. Woomy observes that a model executes 13/19 entry instructions; it does not infer retail bootability from those measurements.

## Réalignement et investigation des indéfinis — 2026-10-02

[LINKING.md](LINKING.md) consigne les observations locales séparément des sources publiques épinglées : Decaf e6c528a20a41c34e0f9eb91dd3da40f119db2dee, Cemu 32e6628a6a9e5255fd0f7e734386a364ff9c106e et WUT tools b22de1a3c0c8950c4ad4b4df170c89c05d3196aa. Les chemins de fixup/relocation consultés laissent une valeur de symbole SHN_UNDEF inchangée ; cette observation ne devient pas une preuve Cafe retail. Aucun code de ces chargeurs n’est repris. Le rapport local conserve les 17 records, les sections, leurs voisins, FILEINFO et les hashes.

[DIRECTION.md](DIRECTION.md) précise l’usage architectural de WiiCompiled/Aurora. Les règles de la passe [SEMANTICS.md](SEMANTICS.md) sont des inférences Woomy testées sur instructions synthétiques : usages d’adresse, offsets et signedness d’opérations, sans identité de structure/classe reconstruite ni sémantique Cafe supplémentaire. Le [pont natif proposé](NATIVE_BRIDGE.md) est une conception Woomy, pas une implémentation ou une ABI officielle de MANMENMI.

## MANMENMI M10 — 2026-10-03

Local source of truth: `C:/Users/yukee/Documents/Manmenmi/Manmenmi-M10-NativeRuntime/Manmenmi`. Inspected integration WOOMY/M10_EXPORT/NATIVE_ABI, validation M10, actual C header/implementation, Cafe process/registry and ABI tests. No source modification or vendoring. This is MANMENMI_MODEL implementation evidence, not a retail reference capture. Native Woomy uses the actual C ABI 1/1/1/1; differences and missing guarantees are recorded in [NATIVE_BRIDGE.md](NATIVE_BRIDGE.md).

The 136-file upstream tree is unchanged. Runtime compilation consumes these 25 pinned files; upstream test compilation additionally consumes `tests/abi_tests.cpp` and `tests/check.hpp`, covered by the complete local integrity snapshot.

| Consumed path | SHA-256 |
| --- | --- |
| `abi/include/manmenmi/abi/manmenmi_abi.h` | `ebe5c248f085875c32667d546589e09042574511e3b1788530ce5ef5d1fbe2f9` |
| `abi/src/manmenmi_abi.cpp` | `3641f0084a6b32cd4d5710a7e0cd4d09d39ecd78459da6c4455ec196412943f6` |
| `cafe/include/manmenmi/cafe/cafe_process.hpp` | `c7372b5cfab3632da1aabe21b9ee5a32cb9799f0ae93357b082fd653e5e368df` |
| `cafe/include/manmenmi/cafe/cafe_runtime.hpp` | `cb43d20a0c767ecc550e0d63a429e108d0f79cf8a0e1dfcbd468b5c81559d340` |
| `cafe/include/manmenmi/cafe/cafe_thread.hpp` | `023159bbb8d53975ecf3c1d34539b109fbbaa9c19f8136bddf4f364569548bce` |
| `cafe/include/manmenmi/cafe/coreinit.hpp` | `a4597f5e6abbf7ee844ab31f4a0e79831ed41add1c53674f6903e298b8fe1556` |
| `cafe/include/manmenmi/cafe/endian.hpp` | `f750ed20657e2d674ad67adfcd98dd1c6558f08bb0d92cf792124272fa893495` |
| `cafe/include/manmenmi/cafe/guest_address.hpp` | `c965d727370f6eab0ec664e6fbfe2d7d8c2ee8bd42b68e08a85445022659878d` |
| `cafe/include/manmenmi/cafe/guest_memory.hpp` | `5302f5aa1c8fb49352994ee3f7d4f88fe5a437d3ee1f733dcf0b8b4139f882a1` |
| `cafe/include/manmenmi/cafe/guest_object_registry.hpp` | `1ecc558587bdad2a70329c9b44ed769b10270f1eb007d283964c8af80470ebad` |
| `cafe/include/manmenmi/cafe/service_registry.hpp` | `3f3401b602d42c0946f84de8b3741f89cb01a72abf3c3243a88a1af76adc95f4` |
| `cafe/include/manmenmi/cafe/version.hpp` | `b3650cd1ec9bc7eeb6ecd520b1c960deebab6bbc449ccbcef73f10916ff5a495` |
| `cafe/src/cafe_process.cpp` | `f79b33674c9e4ba37014b99fe82a3b6d87380dab54e6ad85bc0885028cd8d02c` |
| `cafe/src/cafe_runtime.cpp` | `26340347b44eebbdf96fd42a6c695f3ad8b89437e6272d9d825f46192102311a` |
| `cafe/src/cafe_thread.cpp` | `51bf0de4821d4be7817b53a7f764f15aed5179fee1a12e68fecdab432d531d37` |
| `cafe/src/coreinit.cpp` | `7a4f8115153e6668cdc5209a3aba5a845b979a91c875882384a5fd27832c1e3c` |
| `cafe/src/guest_object_registry.cpp` | `7958493f3c305dfe7839b2d4aeae2469af36791a307498e79750f495c1ab4b4f` |
| `cafe/src/service_registry.cpp` | `a8095728122241c3b5c856f15e93236ba33f445ac60fc250e10f2724eeff6545` |
| `core/include/manmenmi/core/backend.hpp` | `8c7ce6c1f25187696506502f86cc626e777a8ee1ebbfe97e21ee9391d095e140` |
| `core/include/manmenmi/core/config.hpp` | `604b4f21f19ca868f71ee71f55ce144fd8fe95c6af065542f3542bc6ccae0299` |
| `core/include/manmenmi/core/log.hpp` | `a278f82eb0384f7cf8410ac949db715d28e47f41d5ff38c0fac23e3494270d1c` |
| `core/include/manmenmi/core/result.hpp` | `898c9b4e8cf0f7a6b1bcdceb98fc800271a07fac645afef8725762107480e3ee` |
| `core/include/manmenmi/core/runtime.hpp` | `5a1ab368ccf75d960b22bfab2619b7ec284048e7775b26cf31fd5fb889347f63` |
| `core/src/log.cpp` | `91d0211a71d5986faa87bc5e2aca0f4f00c7d64153eb19ba784af1e200d2862f` |
| `core/src/result.cpp` | `46fae0461d73bb3bc53f3e630f0ba2de99522359b6785ac7c5256b5fab160dac` |


## Semantic v3 — 2026-10-04

Les nouvelles règles sont des **WOOMY_INFERRED** dérivées de Typed IR courant et des effets machine existants. Les programmes des tests v3 sont entièrement synthétiques (**SYNTHETIC_VALIDATED**). L'inspection des 115 fonctions et des initialisations mémoire du dump est **EMPIRICALLY_OBSERVED_IN_LOCAL_DUMP**, sans identité de source C++ revendiquée. Aucun nouveau contrat Cafe ni nouveau comportement de chargeur n'est déduit. Les sources M10, leurs versions ABI et leurs empreintes ci-dessus restent gelées. Les algorithmes de propagation/provenance sont locaux au dépôt ; aucune implémentation externe n'a été copiée pour ce jalon.

## Semantic v4 — 4 octobre 2026

- **DOCUMENTED** : [NXP PowerPC Programming Environments, Rev. 1](https://www.nxp.com/docs/en/user-guide/MPCFPE_AD_R1.pdf), §§4.2.4.1.5 et 4.2.4.2, tables de branchement : adresse LR alignée, BO/BI, décrément/test de CTR et continuation. Appliqué uniquement au bclr sans lien reconnu par le décodeur ; aucune particularité Espresso inférée.
- **WOOMY_INFERRED** : effets paramétriques, composition des origines et may-alias dérivés du Typed IR et des plages exactes. Aucun contrat d'import ou de non-aliasing ABI inventé.
- **SYNTHETIC_VALIDATED** : fixtures indépendantes de mémoire, récursion, alias, provenance et comparaisons complètes PPC/IR/natif, notamment branches conditionnelles vers LR et conservation à travers les appels.
- **EMPIRICALLY_OBSERVED_IN_LOCAL_DUMP** : réanalyse locale des 115 fonctions et des 41 sites ; résultats conservés dans les produits dérivés .woomy.
- **MANMENMI_MODEL** reste distinct de **REFERENCE_VALIDATED**. M10 est gelé, aucun code upstream copié ou modifié.
