//! Synthetic instructions only. External tests consume the separately supplied M10 tree.
use std::collections::{BTreeMap, BTreeSet};
use woomy_codegen::{hash, model};
use woomy_ir::{execution::*, runtime::*, *};
use woomy_manmenmi::*;
fn fixture(
    debugger: Option<bool>,
    helper_hash: &str,
) -> (Program, CafeProcessDescriptor, Machine, BTreeMap<u32, u32>) {
    let words = [
        0x7c0802a6, 0x48000101, 0x7c7d1b78, 0x48000101, 0x7c0803a6, 0x38630007, 0x4e800020,
    ];
    let code: BTreeMap<_, _> = words
        .iter()
        .enumerate()
        .map(|(i, w)| (0x1000 + i as u32 * 4, *w))
        .collect();
    let mut decoded: BTreeMap<_, _> = code
        .iter()
        .map(|(pc, w)| (*pc, woomy_ppc::decode(*pc, *w)))
        .collect();
    let mut imports = BTreeMap::new();
    for (pc, symbol) in [
        (0x1004, "OSGetCurrentThread"),
        (0x100c, "OSIsDebuggerInitialized"),
    ] {
        let d = decoded.get_mut(&pc).unwrap();
        d.control_flow = woomy_ppc::Flow::ExternalCall;
        d.operands = vec![woomy_ppc::Operand::ImportSymbol {
            library: "coreinit".into(),
            name: symbol.into(),
        }];
        imports.insert(
            pc,
            Import {
                library: "coreinit".into(),
                symbol: symbol.into(),
            },
        );
    }
    let cfg = woomy_ppc::build_cfg(0x1000, &decoded, &BTreeSet::from([0x1000]), 128);
    let function = lift(
        "synthetic.native",
        "synthetic",
        &"a".repeat(64),
        0x1000,
        &cfg,
        &decoded,
    );
    let image = GuestImageIdentity {
        versions: ImageContextVersions::CURRENT,
        module: "synthetic".into(),
        module_hash: "a".repeat(64),
        image_hash: "b".repeat(64),
    };
    let address = |n| Address32::try_from(n).unwrap();
    let mut process = CafeProcessDescriptor::empty(
        "synthetic.native",
        image.clone(),
        vec![GuestRange {
            start: address(0x1000),
            length: 28,
        }],
        Domain::CafeBootstrapModel,
    );
    process.runtime_artifact_hash = Some(helper_hash.into());
    process.current_thread = Some(Known::model(address(0x10000), "synthetic thread"));
    process.debugger_initialized = debugger.map(|b| Known::model(b, "synthetic debugger"));
    process.threads.push(
        CafeThreadObject::model_zeroed(
            address(0x10000),
            StackBounds {
                low: address(0x20000),
                high: address(0x21000),
                initial_sp: address(0x20f00),
            },
            address(0x1000),
            1,
        )
        .unwrap(),
    );
    let mut state = Machine::new(0, vec![]);
    state.lr = 0xffff0003;
    state.fpr = std::array::from_fn(|n| 0x7ff0000000000001 + n as u64);
    state.fpscr = 0x12345678;
    state.xer = 0x87654321;
    state.guest_memory.regions.push(Region {
        base: 0x1000,
        permissions: READ | EXECUTE,
        kind: 0,
        provenance: "synthetic native code".into(),
        bytes: words.iter().flat_map(|w| w.to_be_bytes()).collect(),
    });
    process.attach(&mut state, &image).unwrap();
    let program = Program {
        image: Some(image),
        runtime_context: Some(process.binding().unwrap()),
        source_barriers: BTreeMap::new(),
        version: RUNTIME_ABI_VERSION,
        entry: 0x1000,
        functions: vec![function],
        imports,
        provider: ProviderKind::ExternalRuntime,
        limits: Limits {
            calls: 32,
            depth: 16,
            trace: 128,
        },
    };
    (program, process, state, code)
}
#[test]
fn model_identity_binding_and_scope_fail_closed() {
    let (p, d, s, _) = fixture(Some(false), &"c".repeat(64));
    let model = model::generate(&p, &d, &s).unwrap();
    assert!(woomy_codegen::generate_program(&p).is_err());
    assert!(woomy_codegen::validation::run(
        std::path::Path::new("missing.exe"),
        &[(s.clone(), 512)]
    )
    .is_err());
    let mut bad = p.clone();
    bad.runtime_context.as_mut().unwrap().module = "other".into();
    assert!(model::generate(&bad, &d, &s).is_err());
    let mut bad = p.clone();
    bad.image.as_mut().unwrap().image_hash = "d".repeat(64);
    assert!(model::generate(&bad, &d, &s).is_err());
    let mut bad = p.clone();
    bad.imports.get_mut(&0x1004).unwrap().symbol = "unsupported".into();
    assert!(model::generate(&bad, &d, &s).is_err());
    let mut unsupported = p.clone();
    unsupported.imports.get_mut(&0x1004).unwrap().symbol = "memcpy".into();
    for op in unsupported.functions[0]
        .blocks
        .iter_mut()
        .flat_map(|b| &mut b.instructions)
    {
        if op.source.address == 0x1004 {
            if let Operation::Call {
                target: CallTarget::Imported { symbol, .. },
                ..
            } = &mut op.operation
            {
                *symbol = "memcpy".into();
            }
        }
    }
    assert!(model::generate(&unsupported, &d, &s).is_err());
    let mut bad = s.clone();
    bad.runtime_context.as_mut().unwrap().provider_abi += 1;
    assert!(model
        .run(std::path::Path::new("missing.exe"), &"0".repeat(64), bad)
        .err()
        .unwrap()
        .to_string()
        .contains("IDENTITY_MISMATCH"));
    let mut bad = s;
    bad.gpr[4] += 1;
    assert!(model
        .run(std::path::Path::new("missing.exe"), &"0".repeat(64), bad)
        .is_err());
    let source = model.generated.source.unwrap();
    assert!(source.contains("mnm_invoke(runtime"));
    assert!(!source.contains("s.gpr[3]+=s.gpr[4]"));
}
#[test]
#[ignore = "requires WOOMY_MANMENMI_SOURCE and WOOMY_MANMENMI_ADAPTER; actual upstream M10"]
fn native_m10_both_profiles_unknown_state_and_full_oracle_agreement() {
    let root = std::path::PathBuf::from(std::env::var_os("WOOMY_MANMENMI_SOURCE").unwrap());
    let helper = std::path::PathBuf::from(std::env::var_os("WOOMY_MANMENMI_ADAPTER").unwrap());
    let helper_hash = hash(&std::fs::read(&helper).unwrap());
    let compiler = woomy_codegen::compiler::NativeCompiler::detect().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let units = [
        "abi/src/manmenmi_abi.cpp",
        "cafe/src/guest_object_registry.cpp",
        "cafe/src/cafe_thread.cpp",
        "cafe/src/cafe_process.cpp",
        "cafe/src/service_registry.cpp",
        "cafe/src/cafe_runtime.cpp",
        "cafe/src/coreinit.cpp",
        "core/src/result.cpp",
        "core/src/log.cpp",
    ]
    .map(|p| root.join(p));
    let includes = [
        root.join("abi/include"),
        root.join("cafe/include"),
        root.join("core/include"),
    ];
    // Run the provided upstream ABI contract suite without modifying or copying
    // its sources; build products live in this test's owned temporary directory.
    let mut abi_includes = includes.to_vec();
    abi_includes.push(root.join("tests"));
    let upstream = compiler
        .build_adapter(
            "#include \"abi_tests.cpp\"\n",
            &abi_includes,
            &units,
            temp.path(),
        )
        .unwrap();
    let bytes = upstream
        .executable
        .unwrap_or_else(|| panic!("upstream ABI tests: {:?}", upstream.record));
    let abi_exe = temp.path().join("upstream_abi.exe");
    std::fs::write(&abi_exe, bytes).unwrap();
    let result = std::process::Command::new(&abi_exe).output().unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let callback_source = format!(
        "{}\n{}\n{}",
        include_str!("../src/runtime.cpp"),
        include_str!("../src/model.cpp"),
        r#"
int main() {
    State s; s.regions.push_back(Region{0x1000,3,4,{'t'},std::vector<uint8_t>(16,0)});
    if(!bounded(s))return 10;
    auto error=fault(Result{s,{},0,0,""},0x1000,"probe",4u);if(error.exit!=4u)return 11;
    static_assert(noexcept(MnmOwner::read(nullptr,0,nullptr,0)));
    static_assert(noexcept(MnmOwner::write(nullptr,0,nullptr,0)));
    uint8_t input[4]={1,2,3,4},output[4]{};
    if(MnmOwner::write(&s,0x1004,input,4)!=MNM_OK || s.writes.size()!=4)return 1;
    if(MnmOwner::read(&s,0x1004,output,4)!=MNM_OK || std::memcmp(input,output,4))return 2;
    for(unsigned i=0;i<4;++i)if(s.writes[i].address!=0x1004+i)return 3;
    s.regions[0].permissions=1;
    if(MnmOwner::write(&s,0x1000,input,4)==MNM_OK || s.writes.size()!=4)return 4;
    if(MnmOwner::read(&s,0xfffffffe,output,4)==MNM_OK)return 5;
    if(MnmOwner::read(&s,0x100f,output,4)==MNM_OK)return 6;
    if(MnmOwner::validate(nullptr,0x1000,4,1)==MNM_OK)return 7;
    if(MnmOwner::validate(&s,0x1000,4,8)==MNM_OK)return 8;
    if(MnmOwner::validate(&s,0x1000,65537,1)==MNM_OK)return 9;
    return 0;
}
"#
    );
    let callbacks = compiler
        .build_adapter(&callback_source, &includes, &units, temp.path())
        .unwrap();
    let bytes = callbacks
        .executable
        .unwrap_or_else(|| panic!("native callbacks: {:?}", callbacks.record));
    let callback_exe = temp.path().join("callbacks.exe");
    std::fs::write(&callback_exe, bytes).unwrap();
    assert!(std::process::Command::new(callback_exe)
        .status()
        .unwrap()
        .success());
    for debugger in [Some(false), Some(true), None] {
        let (p, d, s, code) = fixture(debugger, &helper_hash);
        let mut generated = model::generate(&p, &d, &s).unwrap();
        let build = compiler
            .build_adapter(
                generated.generated.source.as_ref().unwrap(),
                &includes,
                &units,
                temp.path(),
            )
            .unwrap();
        let bytes = build
            .executable
            .unwrap_or_else(|| panic!("{:?}", build.record));
        let exe = temp.path().join("model.exe");
        std::fs::write(&exe, &bytes).unwrap();
        let native = generated
            .run(&exe, &hash(&bytes), s.clone())
            .unwrap()
            .executions
            .remove(0);
        if debugger == Some(false) {
            let identity = generated.identity.clone();
            generated.identity = "0".repeat(64);
            assert!(generated.run(&exe, &hash(&bytes), s.clone()).is_err());
            generated.identity = identity;
        }
        let one = Manmenmi::connect(d.clone(), &helper).unwrap();
        let two = Manmenmi::connect(d, &helper).unwrap();
        let ppc = execute_ppc_program_with_provider(&p, &code, s.clone(), 512, &one);
        let ir = execute_ir_program_with_provider(&p, s, 512, &two);
        assert_eq!(ppc, ir);
        assert_eq!(ir, native);
        if let Some(b) = debugger {
            assert_eq!(native.state.gpr[3], 7 + u32::from(b));
            assert_eq!(native.state.gpr[29], 0x10000);
            assert!(matches!(native.exit, Exit::Returned { .. }));
        } else {
            assert!(matches!(native.exit, Exit::GuestFault { .. }));
            assert_eq!(native.state.gpr[3], 0x10000);
        }
    }
}
