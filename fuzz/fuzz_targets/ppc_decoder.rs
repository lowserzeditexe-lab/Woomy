#![no_main]
libfuzzer_sys::fuzz_target!(|data: (u32,u32)| { let _ = woomy_ppc::decode(data.0,data.1); });
