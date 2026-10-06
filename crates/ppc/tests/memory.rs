use woomy_ppc::{decode, Flow, Operand, Register, Semantics};

#[test]
fn atomic_word_encodings_model_reservation_and_conditional_write() {
    let load = decode(0x1000, 0x7c602028);
    assert_eq!(load.opcode, "lwarx");
    assert_eq!(load.control_flow, Flow::Next);
    let Semantics::Known {
        uses,
        defs,
        memory_read,
        memory_write,
        side_effects,
    } = load.semantics
    else {
        panic!("known atomic load")
    };
    assert!(memory_read && !memory_write);
    assert!(uses.contains(&Register::Gpr(4)));
    assert!(!uses.contains(&Register::Gpr(0)));
    assert!(defs.contains(&Register::Gpr(3)) && defs.contains(&Register::Reservation));
    assert!(side_effects.iter().any(|s| s.contains("asynchronously")));
    let store = decode(0x1004, 0x7c60212d);
    assert_eq!(store.opcode, "stwcx.");
    let Semantics::Known {
        uses,
        defs,
        memory_write,
        ..
    } = store.semantics
    else {
        panic!("known conditional store")
    };
    assert!(memory_write);
    assert!(uses.contains(&Register::Reservation) && uses.contains(&Register::Xer));
    assert!(defs.contains(&Register::Cr(0)) && defs.contains(&Register::Reservation));
    assert_eq!(decode(0, 0x7c602029).status, "INVALID");
    assert_eq!(decode(0, 0x7c60212c).status, "INVALID");
    assert!(woomy_ppc::all_registers().contains(&Register::Reservation));
}

#[test]
fn floating_double_transfers_have_eight_byte_width_and_no_fpscr_conversion() {
    for (op, name, load, update) in [
        (50, "lfd", true, false),
        (51, "lfdu", true, true),
        (54, "stfd", false, false),
        (55, "stfdu", false, true),
    ] {
        let d = decode(0x1000, op << 26 | 3 << 21 | 3 << 16 | 0xfff8);
        assert_eq!(d.opcode, name);
        assert!(matches!(
            d.operands[1],
            Operand::Memory {
                width: 8,
                displacement: -8,
                ..
            }
        ));
        let Semantics::Known {
            uses,
            defs,
            memory_read,
            memory_write,
            ..
        } = d.semantics
        else {
            panic!("known floating transfer")
        };
        assert_eq!(memory_read, load);
        assert_eq!(memory_write, !load);
        assert_eq!(defs.contains(&Register::Gpr(3)), update);
        assert_eq!(defs.contains(&Register::Fpr(3)), load);
        assert_eq!(uses.contains(&Register::Fpr(3)), !load);
        assert!(!defs.contains(&Register::Fpscr));
        let zero_base = decode(0, op << 26 | 3 << 21);
        assert_eq!(zero_base.status, if update { "INVALID" } else { "KNOWN" });
    }
}
