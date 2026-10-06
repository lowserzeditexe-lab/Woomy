#![no_main]
use std::collections::{BTreeMap,BTreeSet};
libfuzzer_sys::fuzz_target!(|words: Vec<u32>| {
    let code:BTreeMap<_,_>=words.iter().take(1024).enumerate().map(|(i,w)|{let a=i as u32*4;(a,woomy_ppc::decode(a,*w))}).collect();
    let cfg=woomy_ppc::build_cfg(0,&code,&BTreeSet::from([0]),1024);
    let _=woomy_ppc::liveness(&cfg,&code);
});
