// Build-time fetch + verify of liblogger.
//
// Process:
// At build time we:
//   1. Pull legal-dlls.csv from the legal-builds repo, find the latest
//      `liblogger_<arch>.so` row, and read its expected SHA-512.
//   2. Download the matching binary from the Toolscreen release.
//   3. Verify SHA-512 against the CSV. If they don't match, fail the build.
//   4. Write the binary to OUT_DIR so liblogger.rs can `include_bytes!` it.
//
// The runtime code never touches the network and never sees a hash literal -
// the entire chain of trust lives in legal-dlls.csv on the legal-builds repo.
//
// Set TUXINJECTOR_LIBLOGGER_REFRESH=1 to force re-download.
// Set TUXINJECTOR_LIBLOGGER_OFFLINE=1 to skip the fetch and embed an empty
//     stub (the runtime will detect this and not load anything).

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const LEGAL_DLLS_URL: &str = "https://raw.githubusercontent.com/Minecraft-Java-Edition-Speedrunning/legal-builds/main/legal-dlls.csv";

const TOOLSCREEN_RELEASE_BASE: &str = "https://github.com/jojoe77777/Toolscreen/releases/download/liblogger-legal";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=TUXINJECTOR_LIBLOGGER_REFRESH");
    println!("cargo:rerun-if-env-changed=TUXINJECTOR_LIBLOGGER_OFFLINE");

    println!("cargo:rerun-if-changed=../../assets/tuxinjector-browser_x64");
    println!("cargo:rerun-if-changed=../../assets/tuxinjector-browser_aarch64");

    let arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let filename = match arch.as_str() {
        "x86_64" => "liblogger_x64.so",
        "x86" => "liblogger_x86.so",
        "aarch64" => "liblogger_arm64.so",
        "arm" => "liblogger_arm32.so",
        other => {
            println!("cargo:warning=tuxinjector: unsupported arch '{other}' for liblogger");
            write_stub();
            return;
        }
    };

    if env::var("TUXINJECTOR_LIBLOGGER_OFFLINE").is_ok() {
        println!("cargo:warning=tuxinjector: TUXINJECTOR_LIBLOGGER_OFFLINE=1, embedding empty stub");
        write_stub();
        return;
    }

    let out_path = out_dir().join("liblogger.so");

    // 1. Fetch legal-dlls.csv and find the expected SHA-512 for our filename.
    let expected_hash = fetch_csv()
        .as_deref()
        .and_then(|csv| find_latest_hash(csv, filename));

    // Escape hatch: point at a local .so and embed that instead of downloading.
    // We still hash-check it against legal-dlls.csv, so the file you pin has to
    // be a legal build. Handy when the rolling `liblogger-legal` release lags
    // the CSV - e.g. CSV legalized 1.0.2 but the release still ships 1.0.1.
    //
    // For testing an unlegalized local build (a dev liblogger that isn't in the
    // CSV yet), set TUXINJECTOR_LIBLOGGER_UNVERIFIED=1 to downgrade the hash
    // mismatch from a hard error to a loud warning. Such a build is NOT legal.
    println!("cargo:rerun-if-env-changed=TUXINJECTOR_LIBLOGGER_PATH");
    println!("cargo:rerun-if-env-changed=TUXINJECTOR_LIBLOGGER_UNVERIFIED");
    if let Ok(path) = env::var("TUXINJECTOR_LIBLOGGER_PATH") {
        let bytes = fs::read(&path)
            .unwrap_or_else(|e| panic!("read TUXINJECTOR_LIBLOGGER_PATH ({path}): {e}"));
        let matches = expected_hash.as_deref() == Some(sha512_hex(&bytes).as_str());
        if !matches && env::var("TUXINJECTOR_LIBLOGGER_UNVERIFIED").is_ok() {
            println!("cargo:warning=tuxinjector: pinned UNVERIFIED liblogger from {path}");
            println!("cargo:warning=tuxinjector: SHA-512 does NOT match legal-dlls.csv -- this build is NOT legal on MCSR Ranked / speedrun.com");
        } else {
            // matches -> prints the verified line; mismatch -> panics with detail
            verify_or_panic(filename, &expected_hash, &bytes);
        }
        fs::write(&out_path, &bytes).expect("write pinned liblogger to OUT_DIR");
        println!("cargo:warning=tuxinjector: pinned liblogger from {path}");
        return;
    }

    // 2. Download the matching binary from the Toolscreen release.
    let url = format!("{TOOLSCREEN_RELEASE_BASE}/{filename}");
    let bytes = match download(&url) {
        Ok(b) => b,
        Err(e) => {
            // Fail loudly — building tuxinjector without liblogger should be
            // an explicit choice (TUXINJECTOR_LIBLOGGER_OFFLINE=1).
            panic!(
                "failed to download {filename} from {url}: {e}\n\
                 set TUXINJECTOR_LIBLOGGER_OFFLINE=1 to build without liblogger\n\
                 (note: any run/match will then be ILLEGAL on speedrun.com / MCSR Ranked)"
            );
        }
    };

    // 3. Verify against the CSV hash if we have one.
    verify_or_panic(filename, &expected_hash, &bytes);

    // 4. Write the binary to OUT_DIR so liblogger.rs can include_bytes! it.
    fs::write(&out_path, &bytes).expect("write liblogger to OUT_DIR");
}

// Hash-check a liblogger binary against the latest legal-dlls.csv entry.
// Mismatch -> panic. No CSV entry at all -> warn and let it through, but that
// build won't be legal on speedrun.com / MCSR Ranked.
fn verify_or_panic(filename: &str, expected_hash: &Option<String>, bytes: &[u8]) {
    match expected_hash {
        Some(expected) => {
            let actual = sha512_hex(bytes);
            if &actual != expected {
                panic!(
                    "{filename} SHA-512 does not match legal-dlls.csv\n\
                     expected: {expected}\n\
                     actual:   {actual}\n\
                     either the Toolscreen release or legal-dlls.csv was just updated;\n\
                     try again, or set TUXINJECTOR_LIBLOGGER_OFFLINE=1 to bypass"
                );
            }
            println!("cargo:warning=tuxinjector: liblogger {filename} verified against legal-dlls.csv");
        }
        None => {
            println!(
                "cargo:warning=tuxinjector: legal-dlls.csv has no entry for {filename} \
                 (PR #27 not merged?), embedding without verification — runs will not be legal"
            );
        }
    }
}

fn out_dir() -> PathBuf {
    PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR not set"))
}

fn write_stub() {
    fs::write(out_dir().join("liblogger.so"), &[][..]).expect("write empty stub");
}

fn fetch_csv() -> Option<String> {
    match Command::new("curl")
        .args(["-sfL", "--max-time", "10", LEGAL_DLLS_URL])
        .output()
    {
        Ok(out) if out.status.success() => String::from_utf8(out.stdout).ok(),
        Ok(out) => {
            println!(
                "cargo:warning=tuxinjector: curl exited {} fetching legal-dlls.csv",
                out.status
            );
            None
        }
        Err(e) => {
            println!("cargo:warning=tuxinjector: failed to spawn curl: {e}");
            None
        }
    }
}

fn download(url: &str) -> Result<Vec<u8>, String> {
    let out = Command::new("curl")
        .args(["-sfL", "--max-time", "60", url])
        .output()
        .map_err(|e| format!("spawn curl: {e}"))?;
    if !out.status.success() {
        return Err(format!("curl exit {}", out.status));
    }
    Ok(out.stdout)
}

/// Parse a `filename,version,sha512` CSV and return the SHA-512 of the
/// highest-versioned row matching the requested filename.
fn find_latest_hash(csv: &str, filename: &str) -> Option<String> {
    let mut latest: Option<(Vec<u32>, String)> = None;
    for line in csv.lines().skip(1) {
        let mut parts = line.splitn(3, ',');
        let name = parts.next()?.trim();
        let ver = parts.next()?.trim();
        let hash = parts.next()?.trim();
        if name != filename {
            continue;
        }
        let ver_parts: Vec<u32> = ver.split('.').filter_map(|s| s.parse().ok()).collect();
        match &latest {
            Some((cur, _)) if ver_parts <= *cur => {}
            _ => latest = Some((ver_parts, hash.to_string())),
        }
    }
    latest.map(|(_, h)| h)
}

fn sha512_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha512};
    let bytes = Sha512::digest(data);
    let mut s = String::with_capacity(128);
    for byte in bytes {
        let _ = write!(s, "{:02x}", byte);
    }
    s
}
