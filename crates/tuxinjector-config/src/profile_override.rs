// Per-instance config-profile override from the wrapper command.

use std::sync::OnceLock;

/// The launch-command profile override, if any. `Some(name)` overrides
/// `active_profile.txt`; an empty `name` means the default profile. `None`
/// means no `--profile` flag was given (fall back to `active_profile.txt`).
pub fn profile_override() -> Option<String> {
    static OVERRIDE: OnceLock<Option<String>> = OnceLock::new();
    OVERRIDE.get_or_init(parse_from_args).clone()
}

fn parse_from_args() -> Option<String> {
    // `--profile <name>` / `--profile=<name>` on the launch command, else the
    // collision-proof TUXINJECTOR_PROFILE env var.
    let mut args = std::env::args_os();
    while let Some(arg) = args.next() {
        let arg = arg.to_string_lossy();
        if let Some(rest) = arg.strip_prefix("--profile=") {
            return Some(rest.trim().to_string());
        }
        if arg == "--profile" {
            // the name is the next argv token (quote it for names with spaces)
            return args.next().map(|n| n.to_string_lossy().trim().to_string());
        }
    }
    std::env::var("TUXINJECTOR_PROFILE").ok().map(|v| v.trim().to_string())
}
