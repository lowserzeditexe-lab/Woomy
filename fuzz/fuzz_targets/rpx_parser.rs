#![no_main]
libfuzzer_sys::fuzz_target!(|data: &[u8]| { let _ = woomy_binary::parse(data); });
