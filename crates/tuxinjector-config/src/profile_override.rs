// Per-instance config-profile override from the wrapper command.

use std::sync::OnceLock;

// `Some(name)` wins over active_profile.txt; an empty name == default profile.
// `None` -> no --profile was passed, so fall back to active_profile.txt.
// Parsed once (argv doesn't change under us) and cached.
pub fn profile_override() -> Option<String> {
    static OVERRIDE: OnceLock<Option<String>> = OnceLock::new();
    OVERRIDE.get_or_init(parse_from_args).clone()
}

fn parse_from_args() -> Option<String> {
    // Accept both `--profile <name>` and `--profile=<name>`. If neither shows up
    // we fall back to the env var, which is less likely to clash with whatever
    // the wrapped game does with its own argv.
    let mut args = std::env::args_os();
    while let Some(arg) = args.next() {
        let arg = arg.to_string_lossy();
        if let Some(rest) = arg.strip_prefix("--profile=") {
            return Some(rest.trim().to_string());
        }
        if arg == "--profile" {
            // name is the next token - caller has to quote it if it has spaces
            return args.next().map(|n| n.to_string_lossy().trim().to_string());
        }
    }
    std::env::var("TUXINJECTOR_PROFILE").ok().map(|v| v.trim().to_string())
}
