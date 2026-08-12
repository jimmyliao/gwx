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
    /// 列出已設定帳號
    Accounts,
    /// 偵測設定:host 可連、gws、已設定帳號(setup 引導)
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

/// The credential host (a secret store). Set GWX_HOST to your ssh alias for it.
fn host() -> String {
    std::env::var("GWX_HOST").unwrap_or_else(|_| "gwx-host".into())
}
fn creds_dir() -> String {
    std::env::var("GWX_CREDS_DIR").unwrap_or_else(|_| "$HOME/gwx-creds".into())
}
/// Base dir holding one per-account gws config dir. Per-account isolation of the
/// token cache / client secret / encrypted store lives here (see run_gws).
fn config_base() -> String {
    std::env::var("GWX_CONFIG_DIR").unwrap_or_else(|_| "$HOME/gws-accounts".into())
}

/// Run a command on the host over ssh, capturing stdout. Err carries stderr / failure.
fn ssh_capture(remote: &str, connect_timeout: u32) -> Result<String, String> {
    let out = Command::new("ssh")
        .args([
            "-o",
            "BatchMode=yes",
            "-o",
            &format!("ConnectTimeout={connect_timeout}"),
            &host(),
            remote,
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Detect setup state and guide the user — a lightweight wizard.
fn run_doctor() -> i32 {
    println!("gwx doctor — 偵測設定\n");
    let h = std::env::var("GWX_HOST").unwrap_or_default();
    if h.is_empty() {
        println!("⚠️  GWX_HOST 未設定 → 用預設 '{}'", host());
        println!("    設定:export GWX_HOST=<你的 credential host 的 ssh alias>");
    } else {
        println!("✅ GWX_HOST = {h}");
    }

    // host 可連?
    match ssh_capture("echo ok", 5) {
        Ok(_) => println!("✅ host 可連(ssh {})", host()),
        Err(e) => {
            println!("❌ host 連不上:{e}");
            println!("    → 檢查 GWX_HOST / ~/.ssh/config / 私有網路(如 Tailscale)是否上線");
            println!("\n(host 連上前無法偵測 gws 與帳號。)");
            return 1;
        }
    }

    // gws 裝了嗎?
    match ssh_capture("command -v gws || echo __MISSING__", 10) {
        Ok(p) if p != "__MISSING__" && !p.is_empty() => println!("✅ gws 已安裝:{p}"),
        _ => println!("❌ host 上找不到 gws → 在 host 執行 `npm i -g @googleworkspace/cli`(或裝 release binary)"),
    }

    // 有哪些帳號 creds?
    let dir = creds_dir();
    match ssh_capture(&format!("ls -1 {dir}/*.json 2>/dev/null | sed 's#.*/##;s#.json##'"), 10) {
        Ok(list) if !list.is_empty() => {
            let accts: Vec<&str> = list.lines().collect();
            println!("✅ 已設定帳號({}):{}", accts.len(), accts.join(", "));
            println!("\n就緒 🎉  試試:gwx mail --as {} list 5", accts[0]);
        }
        _ => {
            println!("⚠️  尚無帳號(host 的 {dir}/ 沒有 *.json)");
            println!("\n下一步:在 host 為每個帳號跑一次 gws OAuth,");
            println!("    scope 只給 gmail.readonly + gmail.compose + drive.readonly(→ draft-only、寄不出),");
            println!("    creds 存成 {dir}/<account>.json");
        }
    }
    0
}

/// Run gws for a given account on the credential host.
/// TODO(productize): swap this ssh call for an HTTP call to the scoped service (see docs/SPEC.md).
fn run_gws(account: &str, args: &[&str]) -> i32 {
    // Per-account isolation: CREDENTIALS_FILE selects the identity, CONFIG_DIR gives
    // each account its own token cache / client secret / encrypted store (else the cache
    // is shared and account A can intermittently act as account B), KEYRING_BACKEND=file
    // avoids the OS keyring that fails on a headless host.
    let remote = format!(
        "GOOGLE_WORKSPACE_CLI_CREDENTIALS_FILE={creds}/{acct}.json \
         GOOGLE_WORKSPACE_CLI_CONFIG_DIR={cfg}/{acct} \
         GOOGLE_WORKSPACE_CLI_KEYRING_BACKEND=file \
         gws {args}",
        creds = creds_dir(),
        cfg = config_base(),
        acct = account,
        args = args.join(" ")
    );
    match Command::new("ssh")
        .args(["-o", "BatchMode=yes", &host(), &remote])
        .status()
    {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("ssh 失敗: {e}");
            1
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
}

/// Bare `gwx` (no subcommand): detect setup state, then either show usage or onboard.
fn run_welcome() -> i32 {
    let accounts: Vec<String> = match ssh_capture(
        &format!("ls -1 {}/*.json 2>/dev/null | sed 's#.*/##;s#.json##'", creds_dir()),
        5,
    ) {
        Ok(list) if !list.trim().is_empty() => list.lines().map(|s| s.to_string()).collect(),
        _ => Vec::new(),
    };

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
        println!("You're not set up yet. Two steps:\n");
        println!("  1. Point gwx at your credential host:");
        println!("       export GWX_HOST=<ssh alias of the host holding your OAuth creds>\n");
        println!("  2. On that host, authorize each Google account (draft-only scopes):");
        println!("       GOOGLE_WORKSPACE_CLI_KEYRING_BACKEND=file \\");
        println!("       GOOGLE_WORKSPACE_CLI_CONFIG_DIR=~/gws-accounts/<account> \\");
        println!("         gws auth login       # scopes: gmail.readonly, gmail.compose, drive.readonly");
        println!("       gws auth export > {}/<account>.json", creds_dir());
        println!("\nThen check it:  gwx doctor       ·       Full help: gwx --help");
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
        Cmd::Accounts => {
            let cmd = format!(
                "ls {}/*.json 2>/dev/null | sed 's#.*/##;s#.json##'",
                creds_dir()
            );
            Command::new("ssh")
                .args(["-o", "BatchMode=yes", &host(), &cmd])
                .status()
                .map(|s| s.code().unwrap_or(1))
                .unwrap_or(1)
        }
        Cmd::Doctor => run_doctor(),
        Cmd::Mail { account, op } => match op {
            MailOp::List { n } => {
                run_gws(&account, &["gmail", "+triage", "--params", &format!("'{{\"maxResults\":{n}}}'")])
            }
            MailOp::Read { id } => {
                run_gws(&account, &["gmail", "+read", "--params", &format!("'{{\"id\":\"{id}\"}}'")])
            }
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
