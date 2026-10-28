//! Claude desktop's Code-tab session list: `claude-code-sessions/<login>/<organisation>/local_*.json`.
//! The desktop app runs each session with its own sign-in, so the folders name it, and
//! `cliSessionId` names the Claude Code session whose log (which carries no account) holds its
//! requests. Only that id and the folder names are read. A Claude account is the organisation,
//! kept as a digest (see [`crate::accounts`]).

use super::ParseOutput;
use crate::model::{claude_account, is_uuid, SessionAccount, Tool};
use serde::Deserialize;
use std::path::Path;

pub const SOURCE: &str = "claude_desktop_session";

#[derive(Deserialize)]
struct Session {
    #[serde(rename = "cliSessionId")]
    cli_session_id: Option<String>,
}

/// A folder named by an id.
fn named(dir: Option<&Path>) -> Option<&str> {
    dir.and_then(Path::file_name).and_then(|n| n.to_str()).filter(|n| is_uuid(n))
}

pub fn parse_file(path: &Path) -> std::io::Result<ParseOutput> {
    let bytes = std::fs::read(path)?;
    let mut out = ParseOutput { next_offset: bytes.len() as u64, lines_total: 1, ..Default::default() };
    let org_dir = path.parent();
    let (Some(org), Some(_)) = (named(org_dir), named(org_dir.and_then(Path::parent))) else {
        out.warn("not inside an account and organisation folder");
        return Ok(out);
    };
    // a session not started yet has no Claude Code session
    let Some(session_id) = serde_json::from_slice::<Session>(&bytes).ok().and_then(|s| s.cli_session_id).filter(|s| !s.is_empty()) else {
        return Ok(out);
    };
    out.sessions.push(SessionAccount { tool: Tool::ClaudeCode, session_id, account: claude_account(org) });
    out.lines_recognised = 1;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCOUNT: &str = "0b5e2c1d-3f4a-4b6c-8d9e-0f1a2b3c4d5e";
    const ORG: &str = "7c8d9e0f-1a2b-4c3d-9e4f-5a6b7c8d9e0f";

    fn write(dir: &Path, rel: &str, body: &str) -> std::path::PathBuf {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn a_session_is_tied_to_the_organisation_it_is_filed_under() {
        let dir = tempfile::tempdir().unwrap();
        let p = write(dir.path(), &format!("{ACCOUNT}/{ORG}/local_1.json"), r#"{"sessionId":"local_1","cliSessionId":"5f1c-cli","title":"private title","cwd":"/x"}"#);
        let out = parse_file(&p).unwrap();
        assert_eq!(out.sessions, vec![SessionAccount { tool: Tool::ClaudeCode, session_id: "5f1c-cli".into(), account: claude_account(ORG) }]);
        assert!(!format!("{:?}", out.sessions).contains("private") && !format!("{:?}", out.sessions).contains(ORG));
    }

    #[test]
    fn a_session_without_a_cli_session_or_outside_the_folders_names_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let fresh = write(dir.path(), &format!("{ACCOUNT}/{ORG}/local_2.json"), r#"{"sessionId":"local_2","cliSessionId":null}"#);
        assert!(parse_file(&fresh).unwrap().sessions.is_empty());
        let damaged = write(dir.path(), &format!("{ACCOUNT}/{ORG}/local_3.json"), "{not json");
        assert!(parse_file(&damaged).unwrap().sessions.is_empty());
        let loose = write(dir.path(), "scheduled/elsewhere/local_4.json", r#"{"cliSessionId":"x"}"#);
        let out = parse_file(&loose).unwrap();
        assert!(out.sessions.is_empty());
        assert_eq!(out.warning_count, 1);
    }
}
