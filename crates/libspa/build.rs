fn main() {
    // FIXME: It would be nice to run this only when tests are run.
    println!("cargo:rerun-if-changed=tests/pod.c");

    // Declare our custom cfg so check-cfg doesn't warn about it.
    println!("cargo:rustc-check-cfg=cfg(has_video_flags_field)");

    let libs = system_deps::Config::new()
        .probe()
        .expect("Cannot find libspa");
    let libspa = libs.get_by_name("libspa").unwrap();

    cc::Build::new()
        .file("tests/pod.c")
        .flag("-Wno-missing-field-initializers")
        .includes(&libspa.include_paths)
        .compile("pod");

    // Detect if spa_video_info_raw has a `flags` field (PipeWire >= 0.3.65).
    // Older PipeWire headers omit it and use int64_t for modifier instead of uint64_t.
    probe_video_flags(&libspa.include_paths);
}

fn probe_video_flags(include_paths: &[std::path::PathBuf]) {
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let probe_c = format!("{}/probe_flags.c", out_dir);
    std::fs::write(
        &probe_c,
        r#"
#include <spa/param/video/raw.h>
void probe(void) {
    struct spa_video_info_raw r;
    r.flags = 0;
    (void)r;
}
"#,
    )
    .unwrap();

    // Use the compiler directly instead of cc::try_compile to suppress
    // noisy "error:" warnings that cc prints to cargo even on expected failures.
    let compiler = cc::Build::new().get_compiler();
    let mut cmd = std::process::Command::new(compiler.path());
    cmd.arg("-c")
        .arg(&probe_c)
        .arg("-o")
        .arg(format!("{}/probe_flags.o", out_dir));
    for path in include_paths {
        cmd.arg(format!("-I{}", path.display()));
    }
    // Suppress all output — we only care about the exit code.
    cmd.stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    if cmd.status().map(|s| s.success()).unwrap_or(false) {
        println!("cargo:rustc-cfg=has_video_flags_field");
    }
}
