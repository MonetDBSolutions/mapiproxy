use std::env;

fn main() {
    // Automatically enable the "oob" feature on Linux.
    //
    // This way we only need to change it in this file when other platforms
    // start to support it.
    let target = env::var("TARGET").expect("cannot get TARGET env var");
    let oob_enabled = target.contains("linux");

    if oob_enabled {
        println!("cargo::rustc-cfg=feature=\"oob\"");
    }
}
