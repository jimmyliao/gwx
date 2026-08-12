//! gwx — 多帳號 × 有治理 × 跨服務的 Google Workspace 層 for AI agents。
//! 引擎 = gws(wrap);gwx 加四層:--as 路由 / policy 治理 / 連結穿透 / scoped serve。
//!
//! Note: run_gws currently shells to `ssh <host> "... gws ..."`. The product target
//! swaps that for an HTTP call to the scoped service (see docs/SPEC.md) so clients hold
//! no token and get no shell on the credential host.

use clap::{Parser, Subcommand};
use std::process::{exit, Command};

/// Work identities: a draft on these requires human review (fallback list).
/// Real deployments list their work identities in policy.yaml — see policy.example.yaml.
const REVIEW_REQUIRED: &[&str] = &["work"];

/// Draft-only OAuth scopes: read Gmail + Drive, create drafts — but NOT gmail.send.
/// The token physically cannot send; this is the backstop behind the policy layer.
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
    },
    /// List configured accounts
    Accounts,
    /// Diagnose setup: is gws installed, which accounts are configured
    Doctor,
}

#[derive(Subcommand)]
enum MailOp {
    /// 未讀摘要(gws gmail +triage)
    List {
        #[arg(default_value_t = 10)]
        n: u32,
    },
    /// 讀某封信
    Read { id: String },
    /// 建草稿(不寄;工作帳號強制 review)
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
    /// 🚫 送信:policy 硬擋,永不自主
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
            println!("   Install it:  npm install -g @googleworkspace/cli   (or a release binary)");
            if remote.is_none() {
                return 1;
            }
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
    let joined = args.join(" ");
    let out = match remote_host() {
        // Remote/fleet mode: CREDENTIALS_FILE selects the identity, CONFIG_DIR isolates,
        // KEYRING_BACKEND=file avoids the OS keyring that's absent on a headless host.
        Some(h) => {
            let remote = format!(
                "GOOGLE_WORKSPACE_CLI_CREDENTIALS_FILE={creds}/{acct}.json \
                 GOOGLE_WORKSPACE_CLI_CONFIG_DIR={cfg}/{acct} \
                 GOOGLE_WORKSPACE_CLI_KEYRING_BACKEND=file \
                 gws {joined}",
                creds = creds_dir(),
                cfg = config_base(),
                acct = account,
            );
            Command::new("ssh")
                .args(["-o", "BatchMode=yes", &h, &remote])
                .output()
        }
        // Local mode (default): run gws on this machine. Per-account CONFIG_DIR; the OS
        // keyring is fine on a workstation, so we don't force the file backend.
        None => Command::new("sh")
            .arg("-c")
            .arg(format!("gws {joined}"))
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
}

/// Run the account OAuth flow. Local mode: opens Google sign-in via gws into a
/// per-account config dir. Remote mode: point the user at the host.
fn run_auth(account: &str) -> i32 {
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
    eprintln!("Opening Google sign-in for '{account}' — read + compose only, never send.\n");
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
    let code = match cmd {
        Cmd::Auth { account } => run_auth(&account),
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
        Cmd::Doctor => run_doctor(),
        Cmd::Mail { account, op } => match op {
            MailOp::List { n } => {
                let max = n.to_string();
                run_gws(&account, &["gmail", "+triage", "--max", &max])
            }
            MailOp::Read { id } => run_gws(&account, &["gmail", "+read", "--id", &id]),
            MailOp::Draft { to, subject, body, cc } => {
                eprintln!("📝 建立草稿於 {account}（不寄）…");
                let raw = build_raw(&to, &subject, &body, cc.as_deref());
                let json = format!("'{{\"message\":{{\"raw\":\"{raw}\"}}}}'");
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
                eprintln!("🚫 送信被 policy 擋下:gwx 不自主寄信(scope 亦不含 gmail.send)。");
                2
            }
        },
        Cmd::Drive { account, op } => match op {
            DriveOp::Ls { query } => run_gws(
                &account,
                &["drive", "files", "list", "--params", &format!("'{{\"q\":\"{query}\",\"pageSize\":20}}'")],
            ),
        },
        Cmd::Doc { account, op } => match op {
            DocOp::Get { file_id } => run_gws(
                &account,
                &["drive", "files", "export", "--params", &format!("'{{\"fileId\":\"{file_id}\",\"mimeType\":\"text/plain\"}}'")],
            ),
            DocOp::Resolve => {
                use std::io::Read;
                let mut text = String::new();
                if std::io::stdin().read_to_string(&mut text).is_err() {
                    eprintln!("讀 stdin 失敗");
                    exit(1);
                }
                let links = extract_links(&text);
                if links.is_empty() {
                    eprintln!("(未在文字中找到 Google Doc/Sheet/Slide/Drive 連結)");
                }
                let mut last = 0;
                for (kind, id, url) in links {
                    println!("── [{kind}] {id}  ({url})");
                    last = run_gws(
                        &account,
                        &["drive", "files", "export", "--params", &format!("'{{\"fileId\":\"{id}\",\"mimeType\":\"text/plain\"}}'")],
                    );
                }
                last
            }
        },
    };
    exit(code);
}
