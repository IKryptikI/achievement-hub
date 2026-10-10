//! Generates the forwarding stubs and the .def file for the DLL this
//! package builds (winmm or version), from proxy/exports/<name>.txt.
use std::fmt::Write;
use std::path::PathBuf;

fn main() {
    let lib = std::env::var("CARGO_PKG_NAME").unwrap().trim_start_matches("proxy-").to_string();
    let root = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../proxy");
    let list = std::fs::read_to_string(root.join("exports").join(format!("{lib}.txt"))).unwrap();
    println!("cargo:rerun-if-changed={}", root.join("exports").display());
    let names: Vec<&str> = list.lines().filter_map(|l| l.split_whitespace().nth(1)).collect();
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());

    let mut def = format!("LIBRARY {lib}\nEXPORTS\n");
    let mut asm = String::new();
    for (i, n) in names.iter().enumerate() {
        writeln!(def, "    {n}=ah_stub_{i}").unwrap();
        writeln!(asm, ".globl _ah_stub_{i}\n_ah_stub_{i}:\n    jmp dword ptr [{{table}} + {}]", i * 4).unwrap();
    }
    std::fs::write(out.join("exports.def"), def).unwrap();
    std::fs::write(out.join("stubs.s"), asm).unwrap();
    let names_rs = format!(
        "pub const LIB: &str = {lib:?};\npub const NAMES: [&str; {}] = [{}];\n",
        names.len(),
        names.iter().map(|n| format!("{:?}", format!("{n}\0"))).collect::<Vec<_>>().join(", ")
    );
    std::fs::write(out.join("names.rs"), names_rs).unwrap();
    println!("cargo:rustc-cdylib-link-arg={}", out.join("exports.def").display());
}
