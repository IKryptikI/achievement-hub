//! Probe: wait for a game to start, prove we can read its memory, and print
//! the exe build so achievement rules can be tied to it.
//!
//!   probe [EXE_NAME...] [--dump ADDR LEN]
//!
//! Defaults to classic Resident Evil 2 (GOG): LeonU.exe / ClaireU.exe.

use std::{env, thread, time::Duration};
use tracker::GameProcess;

fn parse_num(s: &str) -> u64 {
    let s = s.trim_start_matches("0x");
    u64::from_str_radix(s, 16).unwrap_or_else(|_| panic!("not a hex number: {s}"))
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let (names, dump) = match args.iter().position(|a| a == "--dump") {
        Some(i) => (&args[..i], args.get(i + 1..i + 3)),
        None => (&args[..], None),
    };
    let names: Vec<&str> = if names.is_empty() {
        vec!["LeonU.exe", "ClaireU.exe"]
    } else {
        names.iter().map(String::as_str).collect()
    };

    println!("waiting for {} ...", names.join(" / "));
    let game = loop {
        if let Some(g) = GameProcess::find(&names) {
            break g;
        }
        thread::sleep(Duration::from_secs(1));
    };
    println!("found {} (pid {})", game.exe_name(), game.pid());
    println!("module base: {:#x}", game.module_base());

    match game.exe_version() {
        Ok(v) => println!(
            "exe build:   timestamp {:#010x}, image size {:#x}",
            v.timestamp, v.image_size
        ),
        Err(e) => {
            eprintln!("could not read the game's memory: {e}");
            std::process::exit(1);
        }
    }

    if let Some([addr, len]) = dump {
        let (addr, len) = (parse_num(addr), parse_num(len) as usize);
        let mut buf = vec![0u8; len];
        match game.read(addr, &mut buf) {
            Ok(()) => {
                for (i, row) in buf.chunks(16).enumerate() {
                    let hex: Vec<String> = row.iter().map(|b| format!("{b:02x}")).collect();
                    println!("{:#010x}  {}", addr + i as u64 * 16, hex.join(" "));
                }
            }
            Err(e) => eprintln!("dump failed: {e}"),
        }
    }
    println!("memory read OK");
}
