//! All bytes and identities in these fixtures are synthetic.
use woomy_ir::{execution::Machine, runtime::*};
use woomy_manmenmi::*;
fn a(n: u64) -> Address32 {
    Address32::try_from(n).unwrap()
}
fn image() -> GuestImageIdentity {
    GuestImageIdentity {
        versions: ImageContextVersions::CURRENT,
        module: "synthetic".into(),
        module_hash: "a".repeat(64),
        image_hash: "b".repeat(64),
    }
}
fn stack() -> StackBounds {
    StackBounds {
        low: a(0x20000),
        high: a(0x21000),
        initial_sp: a(0x20f00),
    }
}
fn object() -> CafeThreadObject {
    CafeThreadObject::model_zeroed(a(0x10000), stack(), a(0x1000), 0x1234).unwrap()
}
fn process() -> CafeProcessDescriptor {
    let mut p = CafeProcessDescriptor::empty(
        "synthetic-process",
        image(),
        vec![GuestRange {
            start: a(0x1000),
            length: 0x100,
        }],
        Domain::CafeBootstrapModel,
    );
    p.threads.push(object());
    p.current_thread = Some(Known::model(a(0x10000), "test current identity"));
    p.debugger_initialized = Some(Known::model(false, "test debugger state"));
    p
}
#[test]
fn guest_addresses_reject_overflow_without_host_pointer_conversion() {
    assert_eq!(a(u32::MAX as u64).get(), u32::MAX);
    assert!(Address32::try_from(0x1_0000_0000).is_err());
    assert!(a(u32::MAX as u64).offset(1).is_err());
}
#[test]
fn guest_fields_are_encoded_big_endian_at_checked_offsets() {
    let t = object();
    let bytes = &t.chunks[0].bytes;
    assert_eq!(
        &bytes[0..8],
        &[0x4f, 0x53, 0x43, 0x6f, 0x6e, 0x74, 0x78, 0x74]
    );
    assert_eq!(&bytes[0x320..0x324], &[0x74, 0x48, 0x72, 0x44]);
    assert_eq!(&bytes[0x326..0x328], &[0x12, 0x34]);
    assert_eq!(&bytes[0x394..0x398], &[0, 2, 0x10, 0]);
    assert_eq!(&bytes[0x398..0x39c], &[0, 2, 0, 0]);
    assert_eq!(&bytes[0x39c..0x3a0], &[0, 0, 0x10, 0]);
    assert_eq!(bytes.len(), 0x6a0);
    assert!(t.initialization.contains("EXPLICIT_MODEL_ZERO"));
}
#[test]
fn materialization_is_atomic_and_requires_writable_mapping() {
    let t = object();
    let mut memory = GuestMemory {
        regions: vec![Region {
            base: 0x10000,
            permissions: READ,
            kind: 4,
            provenance: "synthetic".into(),
            bytes: vec![0xcc; 0x6a0],
        }],
    };
    let before = memory.clone();
    assert!(t
        .materialize(&mut memory, Domain::CafeBootstrapModel)
        .is_err());
    assert_eq!(memory, before);
    memory.regions[0].permissions = READ | WRITE;
    t.materialize(&mut memory, Domain::CafeBootstrapModel)
        .unwrap();
    assert_eq!(memory.regions[0].bytes, t.chunks[0].bytes);
    memory.regions.clear();
    assert!(t
        .materialize(&mut memory, Domain::CafeBootstrapModel)
        .is_err());
}
#[test]
fn runtime_object_overlap_overflow_and_missing_identity_are_rejected() {
    let mut p = process();
    p.threads.push(object());
    assert_eq!(p.validate(), Err("RUNTIME_OBJECT_OVERLAP"));
    let mut p = process();
    p.threads[0].address.value = a(0xfffffff8);
    assert!(p.validate().is_err());
    let mut p = process();
    p.current_thread.as_mut().unwrap().value = a(0x30000);
    assert_eq!(p.validate(), Err("CURRENT_THREAD_OBJECT_UNAVAILABLE"));
    let mut p = process();
    p.threads[0].address.value = a(0x1000);
    assert_eq!(p.validate(), Err("RUNTIME_OBJECT_OVERLAP"));
}
#[test]
fn process_versions_and_identity_participate_in_binding() {
    let p = process();
    let binding = p.binding().unwrap();
    assert!(binding.valid());
    let mut stale = p.clone();
    stale.versions.thread = 0;
    assert_eq!(stale.validate(), Err("PROCESS_VERSION_MISMATCH"));
    let mut other = p.clone();
    other.debugger_initialized.as_mut().unwrap().value = true;
    assert_ne!(other.identity().unwrap(), p.identity().unwrap());
    other = p.clone();
    other.runtime_artifact_hash = Some("c".repeat(64));
    assert_ne!(other.identity().unwrap(), p.identity().unwrap());
}
#[test]
fn attaching_a_process_does_not_accept_another_image_or_overlapping_memory() {
    let p = process();
    let mut m = Machine::new(0, vec![]);
    let mut wrong = image();
    wrong.image_hash = "c".repeat(64);
    assert_eq!(
        p.attach(&mut m, &wrong),
        Err("PROCESS_IMAGE_IDENTITY_MISMATCH")
    );
    assert!(m.runtime_context.is_none());
    m.guest_memory.regions = p.runtime_memory().unwrap().regions;
    let original = m.clone();
    assert!(p.attach(&mut m, &image()).is_err());
    assert_eq!(m, original);
    m.guest_memory.regions.clear();
    p.attach(&mut m, &image()).unwrap();
    assert_eq!(
        m.runtime_context.as_ref().unwrap().current_thread,
        Some(0x10000)
    );
}
#[test]
fn unknown_process_values_stay_absent_in_json() {
    let p = CafeProcessDescriptor::empty("unknown", image(), vec![], Domain::CafeBootstrapModel);
    p.validate().unwrap();
    let json = serde_json::to_value(p).unwrap();
    for field in [
        "current_core",
        "current_thread",
        "debugger_initialized",
        "stack",
    ] {
        assert!(json[field].is_null());
    }
}
fn reference<T>(value: T) -> Known<T> {
    Known {
        value,
        provenance: ValueOrigin::ReferenceCapture,
        evidence: Evidence {
            class: EvidenceClass::ReferenceCapture,
            source: "synthetic test of reference-format handling".into(),
            detail: "Test bytes only; no actual Cafe capture".into(),
        },
    }
}
#[test]
fn captured_chunks_are_exact_and_unknown_gaps_are_unmapped() {
    let k = reference(a(0x10000));
    let chunk = ThreadChunk {
        offset: 0x320,
        bytes: vec![1, 2, 3, 4],
        provenance: ValueOrigin::ReferenceCapture,
        evidence: k.evidence.clone(),
    };
    let t = CafeThreadObject::captured(k, vec![chunk], None).unwrap();
    let regions = t.regions().unwrap();
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].base, 0x10320);
    assert_eq!(regions[0].bytes, [1, 2, 3, 4]);
    assert!(t.validate(Domain::CafeBootstrapModel).is_err());
    assert!(object()
        .validate(Domain::GameFunctionReferenceContext)
        .is_err());
    let mut p = process();
    p.domain = Domain::GameFunctionReferenceContext;
    assert!(p.validate().is_err());
}
#[test]
fn chunk_overlap_field_proof_and_stack_bounds_are_checked() {
    let mut t = object();
    t.chunks.push(t.chunks[0].clone());
    assert!(t.validate(Domain::CafeBootstrapModel).is_err());
    let mut t = object();
    t.fields[0].bytes[0] ^= 1;
    assert!(t.validate(Domain::CafeBootstrapModel).is_err());
    let mut s = stack();
    s.high = s.low;
    assert!(CafeThreadObject::model_zeroed(a(0x10000), s, a(0x1000), 1).is_err());
}
#[test]
fn import_contracts_keep_unknown_effects_and_test_runtime_separate() {
    for symbol in ["OSGetCurrentThread", "OSIsDebuggerInitialized"] {
        let i = Import {
            library: "coreinit".into(),
            symbol: symbol.into(),
        };
        let c = import_contract(&i).unwrap();
        assert_eq!(c.status, "MODEL_IMPLEMENTED");
        assert!(!c.cafe_unknown_effects.is_empty());
        assert!(c.result.starts_with("r3"));
        assert!(TestRuntime.resolve_import(&i).is_none());
    }
    assert!(import_contract(&Import {
        library: "coreinit".into(),
        symbol: "__ghs_mtx_lock".into()
    })
    .is_none());
}
