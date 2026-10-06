#![no_main]
libfuzzer_sys::fuzz_target!(|data: &[u8]| {
    if data.len() > 1024 * 1024 {
        return;
    }
    if let Ok(module) = woomy_binary::parse(data) {
        if let Ok(image) = woomy_core::cafe::CafeGuestImage::load(
            "synthetic.fuzz",
            &woomy_core::hash_bytes(data),
            "RPX",
            &module,
            data.len(),
        ) {
            let _ = image.verify();
            let _ = image.slice(module.entry_point, 4);
        }
    }
});
