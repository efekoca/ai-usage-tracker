//! Which Claude account a Claude config folder is signed in to: the plan organisation in the
//! `oauthAccount` of its `.claude.json`, kept as a digest. Only that one field is taken; the
//! parser skips the e-mail and names next to it, and nothing else is kept.

use crate::discovery::{claude_config_roots, Env, ExtraPaths};
use crate::model::{claude_account, is_uuid};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Deserialize)]
struct Config {
    #[serde(rename = "oauthAccount")]
    oauth_account: Option<OAuthAccount>,
}

#[derive(Deserialize)]
struct OAuthAccount {
    #[serde(rename = "organizationUuid")]
    organization_uuid: Option<String>,
}

/// Claude Code keeps this file in `CLAUDE_CONFIG_DIR` when that is set, in the home folder for its
/// default folders.
pub fn config_file(env: &Env, root: &Path) -> PathBuf {
    match (&env.claude_config_dir, &env.home) {
        (None, Some(home)) if root == home.join(".claude") || root == home.join(".config").join("claude") => home.join(".claude.json"),
        _ => root.join(".claude.json"),
    }
}

/// Every Claude config folder there is, named as its logs' paths begin, with its settings file.
pub fn roots(env: &Env, extra: &ExtraPaths) -> Vec<(String, PathBuf)> {
    claude_config_roots(env, extra).into_iter().filter(|r| r.is_dir()).map(|r| (crate::accounts::root_key(&r), config_file(env, &r))).collect()
}

/// The account a settings file is signed in to; `Ok(None)` when it names none (signed out, or
/// signed in with an API key).
pub fn signed_in(file: &Path) -> std::io::Result<Option<String>> {
    let cfg: Config = serde_json::from_slice(&std::fs::read(file)?).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    Ok(cfg.oauth_account.and_then(|a| a.organization_uuid).filter(|o| is_uuid(o.trim())).map(|o| claude_account(&o)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORG: &str = "7c8d9e0f-1a2b-4c3d-9e4f-5a6b7c8d9e0f";

    #[test]
    fn only_the_organisation_is_taken_as_a_digest() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join(".claude.json");
        std::fs::write(&f, format!(r#"{{"userID":"u","oauthAccount":{{"accountUuid":"a","emailAddress":"someone@example.com","organizationUuid":"{}","displayName":"X"}},"projects":{{}}}}"#, ORG.to_uppercase())).unwrap();
        assert_eq!(signed_in(&f).unwrap(), Some(claude_account(ORG)));
        std::fs::write(&f, r#"{"projects":{}}"#).unwrap();
        assert_eq!(signed_in(&f).unwrap(), None, "signed out");
        std::fs::write(&f, r#"{"oauthAccount":{"organizationUuid":"not-an-id"}}"#).unwrap();
        assert_eq!(signed_in(&f).unwrap(), None);
        std::fs::write(&f, r#"{"oauthAccount":"#).unwrap();
        assert!(signed_in(&f).is_err(), "a file being written is read again later");
    }

    #[test]
    fn the_default_folders_keep_their_settings_in_the_home_folder() {
        let home = PathBuf::from("/h");
        let env = Env { home: Some(home.clone()), roaming: None, local: None, claude_config_dir: None, codex_home: None, antigravity_data_dir: None, xdg_data_home: None, opencode_db: None };
        assert_eq!(config_file(&env, &home.join(".claude")), home.join(".claude.json"));
        assert_eq!(config_file(&env, Path::new("/work/claude-b")), PathBuf::from("/work/claude-b/.claude.json"));
        let set = Env { claude_config_dir: Some("/h/.claude".into()), ..env };
        assert_eq!(config_file(&set, &home.join(".claude")), home.join(".claude/.claude.json"));
    }
}
