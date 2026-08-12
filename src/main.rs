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
    cmd: Cmd,
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

/// Run gws for a given account on the credential host.
/// TODO(productize): swap this ssh call for an HTTP call to the scoped service (see docs/SPEC.md).
fn run_gws(account: &str, args: &[&str]) -> i32 {
    let remote = format!(
        "GOOGLE_WORKSPACE_CLI_CREDENTIALS_FILE={}/{}.json gws {}",
        creds_dir(),
        account,
        args.join(" ")
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

fn main() {
    let cli = Cli::parse();
    let code = match cli.cmd {
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
