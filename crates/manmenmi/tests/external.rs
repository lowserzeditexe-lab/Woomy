//! Explicit integration with the user's external M10 build; synthetic states only.
use sha2::{Digest, Sha256};
use woomy_ir::{execution::Machine, runtime::*};
use woomy_manmenmi::*;
fn address(n: u64) -> Address32 {
    Address32::try_from(n).unwrap()
}
fn fixture(
    id: &str,
    thread: Option<u64>,
    debugger: Option<bool>,
    hash: &str,
) -> (CafeProcessDescriptor, Machine) {
    let image = GuestImageIdentity {
        versions: ImageContextVersions::CURRENT,
        module: "synthetic".into(),
        module_hash: "a".repeat(64),
        image_hash: "b".repeat(64),
    };
    let mut p = CafeProcessDescriptor::empty(id, image.clone(), vec![], Domain::CafeBootstrapModel);
    p.runtime_artifact_hash = Some(hash.into());
    if let Some(a) = thread {
        p.threads.push(
            CafeThreadObject::model_zeroed(
                address(a),
                StackBounds {
                    low: address(0x80000),
                    high: address(0x81000),
                    initial_sp: address(0x80f00),
                },
                address(0x1000),
                1,
            )
            .unwrap(),
        );
        p.current_thread = Some(Known::model(
            address(a),
            "Explicit synthetic integration input",
        ));
    }
    p.debugger_initialized = debugger.map(|b| Known::model(b, "Explicit synthetic debugger input"));
    let mut m = Machine::new(0, vec![]);
    m.gpr[3] = 0xdeadbeef;
    p.attach(&mut m, &image).unwrap();
    (p, m)
}
#[test]
#[ignore = "requires WOOMY_MANMENMI_ADAPTER pointing to a verified M10 helper; no bundled upstream runtime"]
fn external_m10_process_isolation_errors_debugger_and_memory_callbacks() {
    let exe = std::path::PathBuf::from(
        std::env::var_os("WOOMY_MANMENMI_ADAPTER").expect("explicit M10 adapter path"),
    );
    let hash = format!("{:x}", Sha256::digest(std::fs::read(&exe).unwrap()));
    let (p1, mut s1) = fixture("one", Some(0x10000), Some(false), &hash);
    let (p2, mut s2) = fixture("two", Some(0x20000), Some(true), &hash);
    let one = Manmenmi::connect(p1.clone(), &exe).unwrap();
    let two = Manmenmi::connect(p2, &exe).unwrap();
    let before = s1.clone();
    one.invoke_import(1, &mut s1).unwrap();
    assert_eq!(s1.gpr[3], 0x10000);
    s1.gpr[3] = before.gpr[3];
    assert_eq!(s1, before);
    two.invoke_import(1, &mut s2).unwrap();
    assert_eq!(s2.gpr[3], 0x20000);
    one.invoke_import(2, &mut s1).unwrap();
    assert_eq!(s1.gpr[3], 0);
    two.invoke_import(2, &mut s2).unwrap();
    assert_eq!(s2.gpr[3], 1);
    let before = s2.clone();
    assert_eq!(
        one.invoke_import(1, &mut s2),
        Err("RUNTIME_CONTEXT_OR_PROVIDER_MISMATCH")
    );
    assert_eq!(s2, before);
    s2.runtime_context.as_mut().unwrap().provider_abi += 1;
    assert!(two.invoke_import(1, &mut s2).is_err());
    let (p, mut s) = fixture("missing", None, None, &hash);
    let provider = Manmenmi::connect(p, &exe).unwrap();
    let before = s.clone();
    assert_eq!(
        provider.invoke_import(1, &mut s),
        Err("CURRENT_THREAD_UNAVAILABLE")
    );
    assert_eq!(s, before);
    assert_eq!(
        provider.invoke_import(2, &mut s),
        Err("DEBUGGER_STATE_UNAVAILABLE")
    );
    assert_eq!(s, before);
    let mut removed = s1.clone();
    removed.guest_memory.regions.clear();
    assert_eq!(
        one.invoke_import(1, &mut removed),
        Err("CURRENT_THREAD_MAPPING_UNAVAILABLE")
    );
    let mut bridge = bridge::Bridge::connect(&exe, &hash, &p1).unwrap();
    assert_eq!(bridge.capabilities, vec![(1, 1); 6]);
    let mut m = Machine::new(0, vec![]);
    m.guest_memory.regions = vec![Region {
        base: 0x30000,
        permissions: READ | WRITE,
        kind: 4,
        provenance: "synthetic adapter memory tests".into(),
        bytes: (0..64).collect(),
    }];
    m.gpr[3] = 0x30020;
    m.gpr[4] = 0x30000;
    m.gpr[5] = 8;
    assert_eq!(bridge.invoke(3, &mut m).unwrap(), Some(0x30020));
    assert_eq!(
        &m.guest_memory.regions[0].bytes[32..40],
        &[0, 1, 2, 3, 4, 5, 6, 7]
    );
    assert_eq!(
        m.writes.iter().map(|w| w.address).collect::<Vec<_>>(),
        (0x30020..0x30028).collect::<Vec<_>>()
    );
    // M10 OSBlockMove is explicitly model overlap-safe. Never infer memcpy's overlap contract.
    m.gpr[3] = 0x30001;
    m.gpr[4] = 0x30000;
    m.gpr[5] = 8;
    m.gpr[6] = 0;
    assert_eq!(bridge.invoke(4, &mut m).unwrap(), None);
    assert_eq!(
        &m.guest_memory.regions[0].bytes[1..9],
        &[0, 1, 2, 3, 4, 5, 6, 7]
    );
    m.gpr[3] = 0x30030;
    m.gpr[4] = 0x1234;
    m.gpr[5] = 4;
    assert_eq!(bridge.invoke(5, &mut m).unwrap(), None);
    assert_eq!(&m.guest_memory.regions[0].bytes[48..52], &[0x34; 4]);
    for (dst, permission) in [
        (0xfffffffe, READ | WRITE),
        (0x3003f, READ | WRITE),
        (0x30010, READ),
    ] {
        m.gpr[3] = dst;
        m.gpr[4] = 0x30000;
        m.gpr[5] = 8;
        m.guest_memory.regions[0].permissions = permission;
        let before = m.clone();
        assert!(bridge.invoke(3, &mut m).is_err());
        assert_eq!(m, before);
    }
    assert!(bridge::Bridge::connect(&exe, &"0".repeat(64), &p1).is_err());
}
