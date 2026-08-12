//! gwx — 多帳號 × 有治理 × 跨服務的 Google Workspace 層 for AI agents。
//! 引擎 = gws(wrap);gwx 加四層:--as 路由 / policy 治理 / 連結穿透 / scoped serve。
//!
//! Note: run_gws currently shells to `ssh <host> "... gws ..."`. The product target
//! swaps that for an HTTP call to the scoped service (see docs/SPEC.md) so clients hold
//! no token and get no shell on the credential host.

use clap::{Parser, Subcommand};
use std::io::IsTerminal;
use std::process::{exit, Command};

/// Work identities: a draft on these requires human review (fallback list).
/// Real deployments list their work identities in policy.yaml — see policy.example.yaml.
const REVIEW_REQUIRED: &[&str] = &["work"];

/// OAuth scopes: read Gmail + Drive + create drafts. We deliberately do NOT request
/// gmail.send. NOTE: gmail.compose itself can *technically* send, so the real guarantee
/// is that gwx's code only ever calls drafts.create — never any send endpoint.
const GWX_SCOPES: &str = "https://www.googleapis.com/auth/gmail.readonly,\
https://www.googleapis.com/auth/gmail.compose,\
https://www.googleapis.com/auth/drive.readonly";

/// gwx's registered Google OAuth 'installed app' client_id — ONE app shared by all users
/// so end users never run `gws auth setup` or create their own GCP client. A client_id is
/// public-safe (it travels in every OAuth request), so it lives in source.
const GWX_CLIENT_ID: &str = "145940090997-52bcdtbkmeahde2juknvrss72hbup6fv.apps.googleusercontent.com";
/// The client secret is NOT stored in source (keeps a public repo clean). Release builds
/// inject it at COMPILE time via the `GWX_CLIENT_SECRET` build env; dev builds have none
/// (empty) and fall back to a runtime / BYO client. An installed-app secret is non-confidential
/// for the loopback flow, but build-time injection keeps it out of git regardless.
const GWX_CLIENT_SECRET: &str = match option_env!("GWX_CLIENT_SECRET") {
    Some(s) => s,
    None => "",
};

#[derive(Parser)]
#[command(name = "gwx", version, about = "Multi-account, policy-governed Google Workspace for AI agents (wraps gws).")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Gmail 操作
    Mail {
        #[arg(long = "as")]
        account: String,
        #[command(subcommand)]
        op: MailOp,
    },
    /// Drive 操作
    Drive {
        #[arg(long = "as")]
        account: String,
        #[command(subcommand)]
        op: DriveOp,
    },
    /// Google Doc/Sheet/Slide 操作
    Doc {
        #[arg(long = "as")]
        account: String,
        #[command(subcommand)]
        op: DocOp,
    },
    /// Authorize a Google account (opens sign-in; read + compose, never send)
    Auth {
        /// A name you choose for this account, e.g. "personal" or "work"
        account: String,
        /// Print the sign-in URL to copy instead of opening a browser (headless / agents)
        #[arg(long)]
        show_url: bool,
    },
    /// List configured accounts
    Accounts,
    /// Rename a configured account
    Rename {
        /// Current account name
        old: String,
        /// New account name
        new: String,
    },
    /// Diagnose setup: is gws installed, which accounts are configured
    Doctor,
    /// Install gws (the engine gwx wraps) for this platform
    Setup,
}

#[derive(Subcommand)]
enum MailOp {
    /// Inbox summary — unread by default, or a Gmail search query
    List {
        #[arg(default_value_t = 10)]
        n: u32,
        /// Gmail search query (default: is:unread), e.g. "travel OR 訂房 OR 機票"
        #[arg(long, short = 'q')]
        query: Option<String>,
    },
    /// Read a message
    Read { id: String },
    /// Create a draft (never sends; work accounts require review)
    Draft {
        #[arg(long)]
        to: String,
        #[arg(long)]
        subject: String,
        #[arg(long, default_value = "")]
        body: String,
        #[arg(long)]
        cc: Option<String>,
    },
    /// 🚫 Send — blocked by policy; gwx never sends on its own
    Send,
}

#[derive(Subcommand)]
enum DriveOp {
    /// 列檔(gws drive files list)
    Ls {
        #[arg(default_value = "")]
        query: String,
    },
}

#[derive(Subcommand)]
enum DocOp {
    /// 匯出檔案內容為文字
    Get { file_id: String },
    /// 讀 stdin 抽連結,逐一 export(跨服務穿透)
    Resolve,
}

/// Remote/fleet mode: `Some(host)` if GWX_HOST is a non-empty ssh alias.
/// `None` = local mode — gws and credentials live on this machine (the default,
/// and all a single-machine user ever needs).
fn remote_host() -> Option<String> {
    match std::env::var("GWX_HOST") {
        Ok(h) if !h.trim().is_empty() => Some(h),
        _ => None,
    }
}
fn creds_dir() -> String {
    std::env::var("GWX_CREDS_DIR").unwrap_or_else(|_| "$HOME/gwx-creds".into())
}
/// Base dir holding one per-account gws config dir (per-account isolation of the
/// token cache / secret / encrypted store lives here — see run_gws).
fn config_base() -> String {
    std::env::var("GWX_CONFIG_DIR").unwrap_or_else(|_| "$HOME/.gwx/accounts".into())
}
/// Expand a leading `$HOME` for local (non-shell) use.
fn expand_home(p: &str) -> String {
    match (p.strip_prefix("$HOME"), std::env::var("HOME")) {
        (Some(rest), Ok(home)) => format!("{home}{rest}"),
        _ => p.to_string(),
    }
}
/// POSIX single-quote a string for safe interpolation into a remote shell command.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Run a shell command in the active mode — locally (`sh -c`) or on the remote host
/// (ssh) — capturing stdout. Err carries stderr / failure. Used by doctor/welcome.
fn probe(shell_cmd: &str, connect_timeout: u32) -> Result<String, String> {
    let out = match remote_host() {
        Some(h) => Command::new("ssh")
            .args([
                "-o",
                "BatchMode=yes",
                "-o",
                &format!("ConnectTimeout={connect_timeout}"),
                &h,
                shell_cmd,
            ])
            .output(),
        None => Command::new("sh").arg("-c").arg(shell_cmd).output(),
    }
    .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// List configured accounts (subdirs of the per-account config base), in the active mode.
fn list_accounts() -> Vec<String> {
    match probe(
        &format!("ls -1d {}/*/ 2>/dev/null | sed 's#/*$##;s#.*/##'", config_base()),
        8,
    ) {
        Ok(list) if !list.trim().is_empty() => list.lines().map(|s| s.to_string()).collect(),
        _ => Vec::new(),
    }
}

/// Print the onboarding steps for the active mode.
fn print_onboarding() {
    match remote_host() {
        None => {
            println!("Add your first account (opens Google sign-in; read + compose only, never send):");
            println!("    gwx auth <name>          e.g.  gwx auth personal");
            println!("\nThen:  gwx mail --as personal list");
        }
        Some(_) => {
            println!("You're in remote mode (GWX_HOST is set). Authorize each account ON the host:");
            println!("    GOOGLE_WORKSPACE_CLI_KEYRING_BACKEND=file \\");
            println!("    GOOGLE_WORKSPACE_CLI_CONFIG_DIR={}/<name> \\", config_base());
            println!("      gws auth login --scopes {GWX_SCOPES}");
            println!("\nThen re-check:  gwx doctor");
        }
    }
}

/// Diagnose setup and guide the user. Local by default; mentions the host only in remote mode.
fn run_doctor() -> i32 {
    println!("gwx doctor\n");
    let remote = remote_host();
    let place = if remote.is_some() { "the host" } else { "this machine" };

    // Is gws available?
    match probe("command -v gws || echo __MISSING__", 8) {
        Ok(p) if p != "__MISSING__" && !p.is_empty() => println!("✅ gws found on {place}: {p}"),
        Ok(_) => {
            println!("❌ gws is not installed on {place}.");
            if remote.is_none() {
                println!("   Install it:  gwx setup      (fetches the matching gws binary for your platform)");
                println!("   or manually: brew install googleworkspace-cli  ·  https://github.com/googleworkspace/cli/releases");
                return 1;
            }
            println!("   Install gws on the host (see docs/SPEC.md).");
        }
        Err(e) => {
            // Only reachable in remote mode (ssh itself failed).
            println!("❌ can't reach host '{}': {e}", remote.unwrap_or_default());
            println!("   Check GWX_HOST / ~/.ssh/config / your private network.");
            return 1;
        }
    }

    // Which accounts are set up?
    let accts = list_accounts();
    if let Some(first) = accts.first() {
        println!("✅ Accounts: {}", accts.join(", "));
        println!("\nReady 🎉  Try:  gwx mail --as {first} list 5");
    } else {
        println!("⚠️  No accounts set up yet.\n");
        print_onboarding();
    }
    0
}

/// Run gws for a given account — locally by default, or on the remote host when
/// GWX_HOST is set. Per-account CONFIG_DIR isolates each account's token cache / secret /
/// encrypted store (else account A can intermittently act as account B).
fn run_gws(account: &str, args: &[&str]) -> i32 {
    let out = match remote_host() {
        // Remote/fleet mode: CREDENTIALS_FILE selects the identity, CONFIG_DIR isolates,
        // KEYRING_BACKEND=file avoids the OS keyring that's absent on a headless host.
        // Each arg is shell-escaped so JSON / quotes / spaces survive the remote shell.
        Some(h) => {
            let escaped = args.iter().map(|a| sh_quote(a)).collect::<Vec<_>>().join(" ");
            let remote = format!(
                "GOOGLE_WORKSPACE_CLI_CREDENTIALS_FILE={creds}/{acct}.json \
                 GOOGLE_WORKSPACE_CLI_CONFIG_DIR={cfg}/{acct} \
                 GOOGLE_WORKSPACE_CLI_KEYRING_BACKEND=file \
                 gws {escaped}",
                creds = creds_dir(),
                cfg = config_base(),
                acct = account,
            );
            Command::new("ssh")
                .args(["-o", "BatchMode=yes", &h, &remote])
                .output()
        }
        // Local mode (default): exec gws directly — NO shell, so args (JSON, single quotes,
        // spaces) pass through verbatim and need no escaping. Per-account CONFIG_DIR; the OS
        // keyring is fine on a workstation, so we don't force the file backend.
        None => Command::new("gws")
            .args(args)
            .env(
                "GOOGLE_WORKSPACE_CLI_CONFIG_DIR",
                expand_home(&format!("{}/{}", config_base(), account)),
            )
            .output(),
    };
    let out = match out {
        Ok(o) => o,
        Err(e) => {
            eprintln!("failed to run gws: {e}");
            return 1;
        }
    };
    // Pass the captured streams through unchanged.
    use std::io::Write;
    let _ = std::io::stdout().write_all(&out.stdout);
    let _ = std::io::stderr().write_all(&out.stderr);
    // gws can exit 0 while returning a top-level {"error":{...}} envelope (e.g. a 401
    // authError). Inspect the body, not just the exit code, or auth failures pass as success.
    let stdout = String::from_utf8_lossy(&out.stdout);
    let err_envelope = regex::Regex::new(r#"^\s*\{\s*"error"\s*:"#)
        .unwrap()
        .is_match(&stdout);
    let code = out.status.code().unwrap_or(1);
    if code == 0 && err_envelope {
        eprintln!("gwx: gws returned an error envelope with exit 0 — treating as failure.");
        return 2;
    }
    code
}

/// Export a Google-native file (Doc/Sheet/Slide) to text on STDOUT. `gws export` only writes
/// to a file inside its own working directory (never stdout), so we run it in a temp dir and
/// relay the file to stdout, then clean up.
fn export_doc(account: &str, file_id: &str) -> i32 {
    if file_id.is_empty()
        || !file_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        eprintln!("illegal file id");
        return 1;
    }
    let params = format!("{{\"fileId\":\"{file_id}\",\"mimeType\":\"text/plain\"}}");
    use std::io::Write;
    match remote_host() {
        // Remote: export into a fresh temp dir on the host, cat it, remove it.
        Some(h) => {
            let remote = format!(
                "GOOGLE_WORKSPACE_CLI_CREDENTIALS_FILE={creds}/{acct}.json \
                 GOOGLE_WORKSPACE_CLI_CONFIG_DIR={cfg}/{acct} \
                 GOOGLE_WORKSPACE_CLI_KEYRING_BACKEND=file \
                 sh -c 'd=$(mktemp -d) && cd \"$d\" && gws drive files export --params {p} -o out.txt >/dev/null 2>&1; cat out.txt 2>/dev/null; rm -rf \"$d\"'",
                creds = creds_dir(),
                cfg = config_base(),
                acct = account,
                p = sh_quote(&params),
            );
            match Command::new("ssh")
                .args(["-o", "BatchMode=yes", &h, &remote])
                .output()
            {
                Ok(o) => {
                    let _ = std::io::stdout().write_all(&o.stdout);
                    let _ = std::io::stderr().write_all(&o.stderr);
                    o.status.code().unwrap_or(1)
                }
                Err(e) => {
                    eprintln!("ssh failed: {e}");
                    1
                }
            }
        }
        // Local: run gws inside the OS temp dir (its cwd), then read the output file to stdout.
        None => {
            let dir = std::env::temp_dir();
            let name = format!("gwx-export-{file_id}.txt");
            let out_path = dir.join(&name);
            let res = Command::new("gws")
                .current_dir(&dir)
                .args(["drive", "files", "export", "--params", &params, "-o", &name])
                .env(
                    "GOOGLE_WORKSPACE_CLI_CONFIG_DIR",
                    expand_home(&format!("{}/{}", config_base(), account)),
                )
                .output();
            let code = match res {
                Ok(o) if o.status.success() => match std::fs::read(&out_path) {
                    Ok(bytes) => {
                        let _ = std::io::stdout().write_all(&bytes);
                        0
                    }
                    Err(_) => {
                        // export "succeeded" but wrote no file → surface gws's own output
                        let _ = std::io::stdout().write_all(&o.stdout);
                        let _ = std::io::stderr().write_all(&o.stderr);
                        1
                    }
                },
                Ok(o) => {
                    let _ = std::io::stderr().write_all(&o.stderr);
                    o.status.code().unwrap_or(1)
                }
                Err(e) => {
                    eprintln!("failed to run gws: {e}");
                    1
                }
            };
            let _ = std::fs::remove_file(&out_path);
            code
        }
    }
}

fn review_required(account: &str) -> bool {
    REVIEW_REQUIRED.contains(&account)
}

/// 組 RFC822 → base64url raw(Gmail drafts 用)。draft-only:只產草稿 payload,永不寄。
fn build_raw(to: &str, subject: &str, body: &str, cc: Option<&str>) -> String {
    use base64::{engine::general_purpose::URL_SAFE, Engine};
    let mut m = String::new();
    m.push_str(&format!("To: {to}\r\n"));
    if let Some(c) = cc {
        m.push_str(&format!("Cc: {c}\r\n"));
    }
    m.push_str(&format!("Subject: {subject}\r\n"));
    m.push_str("Content-Type: text/plain; charset=utf-8\r\n\r\n");
    m.push_str(body);
    URL_SAFE.encode(m.as_bytes())
}

/// 從文字(通常是信 body)抽 Google 檔案連結,回傳 (kind, id, url),依 id 去重。
fn extract_links(text: &str) -> Vec<(String, String, String)> {
    use regex::Regex;
    let docs = Regex::new(r"https?://docs\.google\.com/(document|spreadsheets|presentation|forms)/d/([A-Za-z0-9_-]{10,})").unwrap();
    let dfile = Regex::new(r"https?://drive\.google\.com/file/d/([A-Za-z0-9_-]{10,})").unwrap();
    let did = Regex::new(r"https?://drive\.google\.com/[^\s]*[?&]id=([A-Za-z0-9_-]{10,})").unwrap();
    let kind = |s: &str| match s {
        "spreadsheets" => "spreadsheet",
        "presentation" => "presentation",
        "forms" => "form",
        _ => "document",
    }
    .to_string();
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut add = |k: String, id: String, url: String| {
        if seen.insert(id.clone()) {
            out.push((k, id, url));
        }
    };
    for c in docs.captures_iter(text) {
        add(kind(&c[1]), c[2].to_string(), c[0].to_string());
    }
    for c in dfile.captures_iter(text) {
        add("drive".into(), c[1].to_string(), c[0].to_string());
    }
    for c in did.captures_iter(text) {
        add("drive".into(), c[1].to_string(), c[0].to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::URL_SAFE, Engine};

    #[test]
    fn draft_raw_roundtrips() {
        let raw = build_raw("a@b.com", "Hello", "hi there", Some("c@d.com"));
        let bytes = URL_SAFE.decode(&raw).unwrap();
        let s = String::from_utf8(bytes).unwrap();
        assert!(s.contains("To: a@b.com"));
        assert!(s.contains("Cc: c@d.com"));
        assert!(s.contains("Subject: Hello"));
        assert!(s.contains("hi there"));
        // base64url:不含 + 或 /
        assert!(!raw.contains('+') && !raw.contains('/'));
    }

    #[test]
    fn links_extract_dedup_and_normalize() {
        let sample = "\
doc https://docs.google.com/document/d/1AbC_dEf-GhIjKlMnOp/edit?usp=sharing \
sheet https://docs.google.com/spreadsheets/d/2XyZ0123456789abcd/edit#gid=0 \
slides https://docs.google.com/presentation/d/3PpQqRrSsTtUu-vWxYz/edit \
drive https://drive.google.com/file/d/4FiLeIdAbCdEfGhIjK/view \
dup https://docs.google.com/document/d/1AbC_dEf-GhIjKlMnOp/edit";
        let r = extract_links(sample);
        let kinds: Vec<&str> = r.iter().map(|x| x.0.as_str()).collect();
        assert_eq!(r.len(), 4, "dedup doc → 4 unique");
        assert!(kinds.contains(&"document"));
        assert!(kinds.contains(&"spreadsheet")); // normalized from spreadsheets
        assert!(kinds.contains(&"presentation"));
        assert!(kinds.contains(&"drive"));
        assert!(r.iter().all(|x| x.1.len() >= 10)); // no bare-word false positives
    }

    #[test]
    fn gws_target_matches_a_shipped_asset() {
        // On any platform gwx itself builds for, we must be able to name a gws asset.
        let t = gws_target().expect("supported build platform should map to a gws target");
        let known = [
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
            "aarch64-unknown-linux-gnu",
            "aarch64-unknown-linux-musl",
            "x86_64-unknown-linux-gnu",
            "x86_64-unknown-linux-musl",
        ];
        assert!(known.contains(&t.as_str()), "unexpected gws target: {t}");
    }
}

/// Rename a configured account (moves its per-account config dir; gws's key isn't path-bound).
fn run_rename(old: &str, new: &str) -> i32 {
    for n in [old, new] {
        if n.is_empty() || !n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            eprintln!("illegal account name: {n:?}");
            return 1;
        }
    }
    match remote_host() {
        Some(_) => {
            let base = config_base();
            let shell = format!(
                "d=\"{base}\"; [ -d \"$d/{old}\" ] || {{ echo __NOEXIST__; exit 0; }}; \
                 [ -e \"$d/{new}\" ] && {{ echo __EXISTS__; exit 0; }}; \
                 mv \"$d/{old}\" \"$d/{new}\" && echo __OK__"
            );
            match probe(&shell, 8) {
                Ok(o) if o.contains("__OK__") => {
                    println!("✅ renamed '{old}' → '{new}'");
                    0
                }
                Ok(o) if o.contains("__NOEXIST__") => {
                    eprintln!("no account named '{old}' (see: gwx accounts)");
                    1
                }
                Ok(o) if o.contains("__EXISTS__") => {
                    eprintln!("an account named '{new}' already exists");
                    1
                }
                _ => {
                    eprintln!("rename failed");
                    1
                }
            }
        }
        None => {
            let base = expand_home(&config_base());
            let from = std::path::Path::new(&base).join(old);
            let to = std::path::Path::new(&base).join(new);
            if !from.is_dir() {
                eprintln!("no account named '{old}' (see: gwx accounts)");
                return 1;
            }
            if to.exists() {
                eprintln!("an account named '{new}' already exists");
                return 1;
            }
            match std::fs::rename(&from, &to) {
                Ok(_) => {
                    println!("✅ renamed '{old}' → '{new}'");
                    0
                }
                Err(e) => {
                    eprintln!("rename failed: {e}");
                    1
                }
            }
        }
    }
}

/// Run the account OAuth flow. Local mode: opens Google sign-in via gws into a
/// per-account config dir. Remote mode: point the user at the host.
fn run_auth(account: &str, show_url: bool) -> i32 {
    if remote_host().is_some() {
        eprintln!("Remote mode (GWX_HOST is set): authorize accounts on the host, then run `gwx doctor`.");
        print_onboarding();
        return 1;
    }
    let cfg = expand_home(&format!("{}/{}", config_base(), account));

    let user_has_own_client = std::env::var("GOOGLE_WORKSPACE_CLI_CLIENT_ID").is_ok();
    let mut cmd = Command::new("gws");
    cmd.args(["auth", "login", "--scopes", GWX_SCOPES])
        .env("GOOGLE_WORKSPACE_CLI_CONFIG_DIR", &cfg);

    if user_has_own_client {
        // BYO client: let gws inherit GOOGLE_WORKSPACE_CLI_CLIENT_ID (and secret) from env.
    } else if GWX_CLIENT_ID != "REPLACE_WITH_GWX_OAUTH_CLIENT_ID" {
        cmd.env("GOOGLE_WORKSPACE_CLI_CLIENT_ID", GWX_CLIENT_ID)
            .env("GOOGLE_WORKSPACE_CLI_CLIENT_SECRET", GWX_CLIENT_SECRET);
    } else {
        eprintln!(
            "gwx's built-in Google sign-in isn't configured in this build yet.\n\
             Set GOOGLE_WORKSPACE_CLI_CLIENT_ID to your own OAuth client, or use a build that bundles gwx's client."
        );
        return 1;
    }

    // Only reached when we're actually going to authorize.
    // Manual/URL mode when asked, or automatically when there's no interactive terminal
    // (e.g. an agent ran this) — a browser can't open, so gws prints the URL instead.
    let manual = show_url || !std::io::stdin().is_terminal();
    if manual {
        cmd.env("BROWSER", "/usr/bin/false"); // don't try to open a browser; gws prints the URL
        eprintln!("Sign in to '{account}' (read + compose only, never send).");
        eprintln!("👉 Copy the URL below into any browser, authorize, and this finishes automatically — leave it running.\n");
    } else {
        eprintln!("Opening Google sign-in for '{account}' — read + compose only, never send.\n");
    }
    match cmd.status() {
        Ok(s) if s.success() => {
            println!("\n✅ '{account}' authorized.  Try:  gwx mail --as {account} list 5");
            0
        }
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("couldn't run gws (is it installed? `gwx doctor`): {e}");
            1
        }
    }
}

/// Is `gws` runnable on the current PATH?
fn gws_on_path() -> bool {
    Command::new("gws")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// The gws release-asset triple for this platform, or None if we can't auto-install here.
/// Mirrors the six targets gws (and gwx) ship via cargo-dist.
fn gws_target() -> Option<String> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" | "arm64" => "aarch64",
        _ => return None,
    };
    match std::env::consts::OS {
        "macos" => Some(format!("{arch}-apple-darwin")),
        "linux" => {
            // musl (Alpine / static-linked distros) ships its own loader; otherwise glibc.
            let musl = std::path::Path::new("/lib/ld-musl-x86_64.so.1").exists()
                || std::path::Path::new("/lib/ld-musl-aarch64.so.1").exists();
            let libc = if musl { "unknown-linux-musl" } else { "unknown-linux-gnu" };
            Some(format!("{arch}-{libc}"))
        }
        _ => None,
    }
}

/// Where to drop the gws binary: next to the running gwx (same dir → same PATH entry).
fn gws_install_dir() -> std::path::PathBuf {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.to_path_buf();
        }
    }
    let home = std::env::var("HOME").unwrap_or_default();
    std::path::Path::new(&home).join(".cargo/bin")
}

/// Manual install options (everything *other* than `gwx setup`), printed when
/// auto-install is declined, fails, or isn't available for the platform.
fn print_gws_manual() {
    eprintln!("  • macOS:  brew install googleworkspace-cli");
    eprintln!("  • any OS: https://github.com/googleworkspace/cli/releases");
    eprintln!("  • npm:    npm install -g @googleworkspace/cli   (needs Node)");
}

/// Download + verify (sha256) + install the matching gws release binary next to gwx.
/// Shells out to curl/tar/shasum so gwx stays engine-thin (no HTTP/crypto crates).
fn install_gws() -> i32 {
    if remote_host().is_some() {
        eprintln!("Remote mode (GWX_HOST is set): gws runs on the anchor — install it there, not here.");
        return 1;
    }
    let target = match gws_target() {
        Some(t) => t,
        None => {
            eprintln!(
                "gwx: can't auto-install gws for this platform ({} {}).",
                std::env::consts::OS,
                std::env::consts::ARCH
            );
            print_gws_manual();
            return 1;
        }
    };
    let dir = gws_install_dir();
    let url = format!(
        "https://github.com/googleworkspace/cli/releases/latest/download/google-workspace-cli-{target}.tar.gz"
    );
    eprintln!("Installing gws ({target}) → {} …", dir.display());
    let script = format!(
        r#"set -e
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
curl -fsSL {url} -o "$tmp/g.tar.gz"
curl -fsSL {url}.sha256 -o "$tmp/g.sha256"
exp=$(awk '{{print $1}}' "$tmp/g.sha256")
act=$( (sha256sum "$tmp/g.tar.gz" 2>/dev/null || shasum -a 256 "$tmp/g.tar.gz") | awk '{{print $1}}')
[ -n "$exp" ] && [ "$exp" = "$act" ] || {{ echo "gwx: sha256 verification failed" >&2; exit 3; }}
tar -xzf "$tmp/g.tar.gz" -C "$tmp"
bin=$(find "$tmp" -type f -name gws | head -1)
[ -n "$bin" ] || {{ echo "gwx: no gws binary inside the archive" >&2; exit 4; }}
mkdir -p {dir}
install -m 0755 "$bin" {dir}/gws
"#,
        url = url,
        dir = sh_quote(dir.to_string_lossy().as_ref()),
    );
    match Command::new("sh").arg("-c").arg(&script).status() {
        Ok(s) if s.success() => {
            // Make the freshly-installed gws visible to this same process's child calls.
            let d = dir.to_string_lossy().to_string();
            let path = std::env::var("PATH").unwrap_or_default();
            std::env::set_var("PATH", format!("{d}:{path}"));
            eprintln!("✅ gws installed → {}/gws", dir.display());
            0
        }
        Ok(s) => {
            let code = s.code().unwrap_or(1);
            eprintln!("gwx: gws install failed (exit {code}). Install it yourself:");
            print_gws_manual();
            code
        }
        Err(e) => {
            eprintln!("gwx: couldn't run the installer ({e}). Install gws yourself:");
            print_gws_manual();
            1
        }
    }
}

/// Local-mode preflight: ensure the gws engine exists, offering to install it if not.
/// Returns true if gws is (now) available; false means the caller should abort.
/// Fleet mode is a no-op here — gws lives on the anchor, reached over ssh.
fn ensure_gws_local() -> bool {
    if remote_host().is_some() || gws_on_path() {
        return true;
    }
    eprintln!("gwx needs its engine — gws (the Google Workspace CLI) — which isn't installed yet.");
    // Only auto-install with a human present; an agent/non-TTY gets a copy-paste path instead.
    let interactive = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    if interactive {
        eprint!("Install it now? [Y/n] ");
        use std::io::Write;
        let _ = std::io::stderr().flush();
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        let ans = line.trim().to_lowercase();
        if ans.is_empty() || ans == "y" || ans == "yes" {
            return install_gws() == 0 && gws_on_path();
        }
        eprintln!("\nOK — install it yourself, then re-run:");
        print_gws_manual();
        false
    } else {
        eprintln!("\nRun:  gwx setup      (installs the matching gws for your platform)");
        eprintln!("or install it yourself:");
        print_gws_manual();
        false
    }
}

/// Bare `gwx` (no subcommand): detect setup, then either show usage or onboard.
fn run_welcome() -> i32 {
    let accounts = list_accounts();
    if let Some(first) = accounts.first() {
        println!("gwx — you're set up ✅");
        println!("Accounts: {}", accounts.join(", "));
        println!("\nTry:");
        println!("  gwx mail  --as {first} list 5");
        println!("  gwx drive --as {first} ls");
        println!("  gwx doc   --as {first} get <fileId>");
        println!("\nFull help: gwx --help    ·    Diagnostics: gwx doctor");
        0
    } else {
        println!("gwx — multi-account, policy-governed Google Workspace for AI agents.\n");
        println!("Not set up yet.\n");
        print_onboarding();
        println!("\nDiagnostics: gwx doctor    ·    Full help: gwx --help");
        1
    }
}

fn main() {
    let cli = Cli::parse();
    let cmd = match cli.cmd {
        Some(c) => c,
        None => exit(run_welcome()),
    };
    // Local mode needs the gws engine on this machine; fleet mode runs gws on the anchor.
    // Gate the gws-backed commands once here so a missing engine guides/installs instead of
    // dumping a raw "No such file or directory" from the child process.
    let needs_engine = matches!(
        cmd,
        Cmd::Mail { .. } | Cmd::Drive { .. } | Cmd::Doc { .. } | Cmd::Auth { .. }
    );
    if needs_engine && remote_host().is_none() && !ensure_gws_local() {
        exit(1);
    }
    let code = match cmd {
        Cmd::Auth { account, show_url } => run_auth(&account, show_url),
        Cmd::Accounts => {
            let accts = list_accounts();
            if accts.is_empty() {
                println!("No accounts yet. Add one:  gwx auth <name>");
            } else {
                for a in &accts {
                    println!("{a}");
                }
            }
            0
        }
        Cmd::Rename { old, new } => run_rename(&old, &new),
        Cmd::Doctor => run_doctor(),
        Cmd::Setup => install_gws(),
        Cmd::Mail { account, op } => match op {
            MailOp::List { n, query } => {
                let max = n.to_string();
                let mut args: Vec<&str> = vec!["gmail", "+triage", "--max", &max];
                if let Some(q) = &query {
                    args.push("--query");
                    args.push(q);
                }
                run_gws(&account, &args)
            }
            MailOp::Read { id } => run_gws(&account, &["gmail", "+read", "--id", &id]),
            MailOp::Draft { to, subject, body, cc } => {
                eprintln!("📝 建立草稿於 {account}（不寄）…");
                let raw = build_raw(&to, &subject, &body, cc.as_deref());
                let json = format!("{{\"message\":{{\"raw\":\"{raw}\"}}}}");
                let code = run_gws(&account, &["gmail", "users", "drafts", "create", "--json", &json]);
                if code == 0 {
                    if review_required(&account) {
                        eprintln!("🔴 REVIEW REQUIRED（{account} 為工作身分）：草稿已建於 Gmail 草稿匣,務必由 Jimmy 過目。未經 review = 任務未完成。");
                    } else {
                        eprintln!("✅ 草稿已建於 Gmail 草稿匣（未寄出）。");
                    }
                }
                code
            }
            MailOp::Send => {
                eprintln!("🚫 Send is blocked: gwx only creates drafts and never calls a send endpoint (and doesn't request gmail.send).");
                2
            }
        },
        Cmd::Drive { account, op } => match op {
            DriveOp::Ls { query } => run_gws(
                &account,
                &["drive", "files", "list", "--params", &format!("{{\"q\":\"{query}\",\"pageSize\":20}}")],
            ),
        },
        Cmd::Doc { account, op } => match op {
            DocOp::Get { file_id } => export_doc(&account, &file_id),
            DocOp::Resolve => {
                use std::io::Read;
                let mut text = String::new();
                if std::io::stdin().read_to_string(&mut text).is_err() {
                    eprintln!("failed to read stdin");
                    exit(1);
                }
                let links = extract_links(&text);
                if links.is_empty() {
                    eprintln!("(no Google Doc/Sheet/Slide/Drive links found in the text)");
                }
                let mut last = 0;
                for (kind, id, url) in links {
                    println!("── [{kind}] {id}  ({url})");
                    last = export_doc(&account, &id);
                }
                last
            }
        },
    };
    exit(code);
}
