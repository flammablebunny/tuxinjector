fn main() {
    // rebuild when embedded assets change (include_bytes! doesn't track these)
    println!("cargo:rerun-if-changed=../../assets/tuxinjector-browser_x64");
    println!("cargo:rerun-if-changed=../../assets/tuxinjector-browser_aarch64");
    println!("cargo:rerun-if-changed=../../assets/liblogger_x64.so");
    println!("cargo:rerun-if-changed=../../assets/liblogger_x86.so");
    println!("cargo:rerun-if-changed=../../assets/liblogger_arm64.so");
    println!("cargo:rerun-if-changed=../../assets/liblogger_arm32.so");
}
