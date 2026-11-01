//! Admin subcommands run before the server starts: `create-owner`, `invite`, `reset-password`,
//! `loadtest-accounts`.
//! Each prints its result on stdout; links and tokens are never logged.

use std::collections::HashMap;

use vesper_protocol::Role;
use vesper_store::{NewAccount, NewSession, RESET_TTL_MS, Store, now_ms, random_hex};

use crate::auth::new_token;
use crate::config::Config;

pub const USAGE: &str = "usage: vesper-server [command]
  (no command)              run the server
  create-owner              print a one-use owner invite link, valid 24 h (only while no owner exists)
  invite [--role member|moderator] [--uses N] [--hours H]
                            print an invite link (default: member, 1 use, 168 h)
  reset-password <username> print a one-use link that sets a new password, valid 24 h; any account,
                            the owner too. It replaces an older link and ends every session on use.
  loadtest-accounts --count N [--force-nonempty yes]
                            create N member accounts with one session each and print one token per
                            line. Throwaway data dir only: these accounts log in with no password.
                            Refuses a data dir that already has accounts unless --force-nonempty yes.
  help                      print this";

const HOUR_MS: i64 = 3600 * 1000;
/// Keeps `load_<i>_<hex>` inside the 24-char username limit with room to spare.
const MAX_LOAD_ACCOUNTS: u64 = 10_000;

/// `args` are the command line without the program name. `Ok(None)`: no subcommand, serve.
pub async fn run(args: &[String], cfg: &Config) -> Result<Option<String>, String> {
    let Some((cmd, rest)) = args.split_first() else {
        return Ok(None);
    };
    // The one positional argument: everything else is `--flag value` pairs.
    if cmd == "reset-password" {
        let [name] = rest else {
            return Err(format!("reset-password needs one username\n{USAGE}"));
        };
        return reset_link(&open(cfg).await?, cfg, name).await.map(Some);
    }
    let flags = flags(rest)?;
    let allow = |known: &[&str]| match flags.keys().find(|k| !known.contains(&k.as_str())) {
        Some(k) => Err(format!("{cmd}: unknown flag {k}\n{USAGE}")),
        None => Ok(()),
    };
    match cmd.as_str() {
        "create-owner" => {
            allow(&[])?;
            let store = open(cfg).await?;
            if store.owner_exists().await.map_err(err)? {
                return Err("an owner already exists".into());
            }
            invite_link(&store, cfg, Role::Owner, 1, 24).await.map(Some)
        }
        "invite" => {
            allow(&["--role", "--uses", "--hours"])?;
            let role = match flags.get("--role").map(String::as_str) {
                None | Some("member") => Role::Member,
                Some("moderator") => Role::Moderator,
                Some(r) => return Err(format!("--role {r}: must be member or moderator")),
            };
            let uses = num(&flags, "--uses", 1)?;
            let hours = num(&flags, "--hours", 168)?;
            let uses = u32::try_from(uses).map_err(|_| "--uses is too large")?;
            let store = open(cfg).await?;
            invite_link(&store, cfg, role, uses, hours).await.map(Some)
        }
        "loadtest-accounts" => {
            allow(&["--count", "--force-nonempty"])?;
            if !flags.contains_key("--count") {
                return Err(format!("loadtest-accounts needs --count N\n{USAGE}"));
            }
            let count = num(&flags, "--count", 0)?;
            if count > MAX_LOAD_ACCOUNTS {
                return Err(format!("--count must be at most {MAX_LOAD_ACCOUNTS}"));
            }
            let store = open(cfg).await?;
            if !flags.contains_key("--force-nonempty")
                && !store.users().await.map_err(err)?.is_empty()
            {
                return Err("this data dir already has accounts; use a throwaway one (or pass --force-nonempty yes)".into());
            }
            let mut tokens = Vec::new();
            for i in 0..count {
                let name = format!("load_{i}_{}", random_hex(2));
                let user = store
                    .create_account(NewAccount {
                        username: &name,
                        display_name: &name,
                        password_phc: None,
                        role: Role::Member,
                    })
                    .await
                    .map_err(err)?;
                let (token, token_hash) = new_token();
                let s = NewSession {
                    token_hash,
                    label: "load test",
                    now: now_ms(),
                };
                store.create_session(&user.user_id, s).await.map_err(err)?;
                tokens.push(token);
            }
            Ok(Some(tokens.join("\n")))
        }
        "help" | "--help" | "-h" => Ok(Some(USAGE.into())),
        other => Err(format!("unknown command {other}\n{USAGE}")),
    }
}

async fn open(cfg: &Config) -> Result<Store, String> {
    let path = cfg.data_dir.join("vesper.db");
    Store::open(&path)
        .await
        .map_err(|e| format!("open {}: {e}", path.display()))
}

async fn invite_link(
    store: &Store,
    cfg: &Config,
    role: Role,
    uses: u32,
    hours: u64,
) -> Result<String, String> {
    let ttl = i64::try_from(hours)
        .ok()
        .and_then(|h| h.checked_mul(HOUR_MS))
        .ok_or("--hours is too large")?;
    let (code, hash) = new_token();
    store
        .create_invite(&hash, role, uses, now_ms().saturating_add(ttl), None)
        .await
        .map_err(err)?;
    Ok(format!("{}/invite#{code}", cfg.origin))
}

async fn reset_link(store: &Store, cfg: &Config, name: &str) -> Result<String, String> {
    let row = store
        .login_row(&name.to_lowercase())
        .await
        .map_err(err)?
        .ok_or(format!("no account named {name}"))?;
    let (code, hash) = new_token();
    store
        .create_reset(&row.user_id, &hash, now_ms() + RESET_TTL_MS, None)
        .await
        .map_err(err)?;
    Ok(format!("{}/reset#{code}", cfg.origin))
}

fn flags(args: &[String]) -> Result<HashMap<String, String>, String> {
    let mut m = HashMap::new();
    let mut it = args.iter();
    while let Some(flag) = it.next() {
        let v = it.next().ok_or(format!("{flag} needs a value"))?;
        m.insert(flag.clone(), v.clone());
    }
    Ok(m)
}

/// A whole number of at least 1; `default` when the flag is absent.
fn num(m: &HashMap<String, String>, k: &str, default: u64) -> Result<u64, String> {
    let Some(v) = m.get(k) else {
        return Ok(default);
    };
    match v.parse::<u64>() {
        Ok(n) if n >= 1 => Ok(n),
        _ => Err(format!("{k} {v}: must be a whole number of at least 1")),
    }
}

fn err(e: vesper_store::StoreError) -> String {
    e.to_string()
}
