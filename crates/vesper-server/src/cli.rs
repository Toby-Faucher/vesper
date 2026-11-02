//! Admin subcommands run before the server starts: `create-owner`, `invite`, `reset-password`,
//! `loadtest-accounts`, `backup`. Each prints its result on stdout; links and tokens are never logged.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

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
  backup <dir>              write vesper-<UTC time>.db (a consistent copy, safe while the server runs)
                            and vesper-<UTC time>-uploads/ into <dir>. Never overwrites.
  help                      print this";

const HOUR_MS: i64 = 3600 * 1000;
/// Keeps `load_<i>_<hex>` inside the 24-char username limit with room to spare.
const MAX_LOAD_ACCOUNTS: u64 = 10_000;

/// `args` are the command line without the program name. `Ok(None)`: no subcommand, serve.
pub async fn run(args: &[String], cfg: &Config) -> Result<Option<String>, String> {
    let Some((cmd, rest)) = args.split_first() else {
        return Ok(None);
    };
    // The one-positional-argument commands: everything else is `--flag value` pairs.
    if cmd == "reset-password" {
        let [name] = rest else {
            return Err(format!("reset-password needs one username\n{USAGE}"));
        };
        return reset_link(&open(cfg).await?, cfg, name).await.map(Some);
    }
    if cmd == "backup" {
        let [dir] = rest else {
            return Err(format!("backup needs one directory\n{USAGE}"));
        };
        return backup(cfg, Path::new(dir), now_ms()).await.map(Some);
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

/// `backup <dir>` with the clock passed in, so a test can pin the file names.
pub async fn backup(cfg: &Config, dir: &Path, now: i64) -> Result<String, String> {
    let w = write_backup(cfg, dir, &format!("vesper-{}", utc_stamp(now)), true, None).await?;
    let (up, n) = w.uploads.unwrap_or_default();
    Ok(format!(
        "backup written to {}\n  database: {} ({} bytes)\n  uploads:  {} ({} files, {} bytes; {} vanished during the copy and were skipped)\nIt holds password and session hashes: keep it private.",
        dir.display(),
        w.db.display(),
        w.db_bytes,
        up.display(),
        n.files,
        n.bytes,
        n.skipped,
    ))
}

/// What one backup wrote.
pub struct Written {
    pub db: PathBuf,
    pub db_bytes: u64,
    uploads: Option<(PathBuf, Copied)>,
}

/// `<stem>.db`, then `<stem>-uploads/` when `uploads`, then `<stem>.json` when `manifest` is set,
/// into `dir` (made 0700). The database copy comes first, then the uploads: a file uploaded
/// meanwhile is an extra file the restored server's sweep removes, and a file deleted meanwhile
/// is counted as skipped. On any error after the first file is made, every output is removed:
/// what is left on disk is complete.
async fn write_backup(
    cfg: &Config,
    dir: &Path,
    stem: &str,
    uploads: bool,
    manifest: Option<String>,
) -> Result<Written, String> {
    let path = cfg.data_dir.join("vesper.db");
    // Not `open`: a wrong data dir must fail, not create, migrate and back up an empty database.
    if !path.is_file() {
        return Err(format!(
            "{} not found; is VESPER_DATA_DIR right?",
            path.display()
        ));
    }
    let store = Store::open_existing(&path)
        .await
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    let src = cfg.data_dir.join("uploads");
    private_dir(dir, true).map_err(io(dir))?;
    // A backup dir inside uploads/ would be copied into the next backup as a shard.
    if let (Ok(d), Ok(u)) = (dir.canonicalize(), src.canonicalize())
        && d.starts_with(&u)
    {
        return Err(format!(
            "{} is inside {}; pick a dir outside it",
            dir.display(),
            src.display()
        ));
    }
    let db = dir.join(format!("{stem}.db"));
    let up = dir.join(format!("{stem}-uploads"));
    let man = dir.join(format!("{stem}.json"));
    for p in [&db, &up, &man] {
        if p.symlink_metadata().is_ok() {
            return Err(format!("{} exists; not overwriting", p.display()));
        }
    }
    // Created empty with mode 600 first: VACUUM INTO writes into an empty file and keeps its mode.
    private_file(&db).map_err(io(&db))?;
    let written = async {
        store
            .backup_into(&db)
            .await
            .map_err(|e| format!("{}: {e}", db.display()))?;
        let mut copied = None;
        if uploads {
            let mut n = Copied::default();
            private_dir(&up, false).map_err(io(&up))?;
            copy_uploads(&src, &up, &mut n).map_err(io(&up))?;
            copied = Some((up.clone(), n));
        }
        if let Some(text) = &manifest {
            private_file(&man)
                .and_then(|mut f| std::io::Write::write_all(&mut f, text.as_bytes()))
                .map_err(io(&man))?;
        }
        let db_bytes = std::fs::metadata(&db).map(|m| m.len()).map_err(io(&db))?;
        Ok::<_, String>(Written {
            db: db.clone(),
            db_bytes,
            uploads: copied,
        })
    }
    .await;
    drop(store);
    written.map_err(|e| {
        let _ = std::fs::remove_file(&db);
        let _ = std::fs::remove_dir_all(&up);
        let _ = std::fs::remove_file(&man);
        format!("{e}; the partial backup was removed")
    })
}

/// The two kinds of automatic backup in `<data>/backups`. Each keeps its own newest
/// `backup_keep`, so hourly ones never push out the copy taken before a migration.
#[derive(Clone, Copy, Debug)]
pub enum Auto {
    /// At start, before the server migrates the database from schema `from` to `to`.
    PreMigration { from: i64, to: i64 },
    /// Every `VESPER_BACKUP_EVERY` hours.
    Scheduled,
}

impl Auto {
    fn prefix(self) -> &'static str {
        match self {
            Auto::PreMigration { .. } => "pre-",
            Auto::Scheduled => "auto-",
        }
    }
}

/// An automatic backup into `<data>/backups`: `pre-<from>-to-<to>-<UTC time>` or
/// `auto-<UTC time>`, the database and a small JSON manifest, plus the uploads only with
/// `VESPER_BACKUP_UPLOADS=1` (they can be large). Then the oldest of that kind past
/// `backup_keep` are deleted. Returns the database copy's path.
pub async fn auto_backup(cfg: &Config, kind: Auto, now: i64) -> Result<PathBuf, String> {
    let dir = cfg.data_dir.join("backups");
    let stamp = utc_stamp(now);
    let (stem, from, to) = match kind {
        Auto::PreMigration { from, to } => (format!("pre-{from:04}-to-{to:04}-{stamp}"), from, to),
        Auto::Scheduled => (format!("auto-{stamp}"), 0, 0),
    };
    let manifest = serde_json::json!({
        "kind": match kind { Auto::PreMigration { .. } => "pre-migration", Auto::Scheduled => "scheduled" },
        "time": stamp,
        "server_version": crate::VERSION,
        "server_build": cfg.build,
        "schema_from": from,
        "schema_to": to,
        "uploads": cfg.backup_uploads,
    });
    let w = write_backup(
        cfg,
        &dir,
        &stem,
        cfg.backup_uploads,
        Some(format!("{manifest:#}\n")),
    )
    .await?;
    prune(&dir, kind.prefix(), cfg.backup_keep).map_err(io(&dir))?;
    Ok(w.db)
}

/// Deletes all but the newest `keep` backups whose name starts with `prefix`: their `.db`,
/// `.json` and `-uploads/`. Newest by the UTC time that ends every name.
fn prune(dir: &Path, prefix: &str, keep: usize) -> std::io::Result<()> {
    let mut stems: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter_map(|n| Some(n.strip_suffix(".db")?.to_owned()))
        .filter(|n| n.starts_with(prefix) && n.len() >= 16)
        .collect();
    stems.sort_by(|a, b| a[a.len() - 16..].cmp(&b[b.len() - 16..]));
    let old = stems.len().saturating_sub(keep);
    for stem in &stems[..old] {
        std::fs::remove_file(dir.join(format!("{stem}.db")))?;
        let _ = std::fs::remove_file(dir.join(format!("{stem}.json")));
        let _ = std::fs::remove_dir_all(dir.join(format!("{stem}-uploads")));
    }
    Ok(())
}

#[derive(Default)]
struct Copied {
    files: u64,
    bytes: u64,
    skipped: u64,
}

/// Error mapper naming the path.
fn io(what: &Path) -> impl Fn(std::io::Error) -> String + '_ {
    move |e| format!("{}: {e}", what.display())
}

/// `uploads/<2 hex>/<id>` into `dest`, skipping `uploads/tmp` (files still being received).
fn copy_uploads(src: &Path, dest: &Path, n: &mut Copied) -> std::io::Result<()> {
    let shards = match std::fs::read_dir(src) {
        Ok(d) => d,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    for shard in shards {
        let shard = shard?;
        if shard.file_name() == "tmp" || !shard.file_type()?.is_dir() {
            continue;
        }
        let files = match std::fs::read_dir(shard.path()) {
            Ok(d) => d,
            Err(e) if e.kind() == ErrorKind::NotFound => continue,
            Err(e) => return Err(e),
        };
        let out = dest.join(shard.file_name());
        private_dir(&out, false)?;
        for f in files {
            let f = f?;
            if !f.file_type()?.is_file() {
                continue;
            }
            let mut from = match std::fs::File::open(f.path()) {
                Ok(f) => f,
                Err(e) if e.kind() == ErrorKind::NotFound => {
                    n.skipped += 1;
                    continue;
                }
                Err(e) => return Err(e),
            };
            let mut to = private_file(&out.join(f.file_name()))?;
            n.bytes += std::io::copy(&mut from, &mut to)?;
            n.files += 1;
        }
    }
    Ok(())
}

/// Mode 700. `parents`: create missing parents and accept an existing dir (its mode is kept).
fn private_dir(dir: &Path, parents: bool) -> std::io::Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(parents);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(dir)
}

/// A new file with mode 600; an existing one is an error.
fn private_file(path: &Path) -> std::io::Result<std::fs::File> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o.open(path)
}

/// `20261101T203000Z` for a Unix time in milliseconds.
pub fn utc_stamp(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's days_from_civil, inverted).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let (h, mi, s) = (rem / 3600, rem % 3600 / 60, rem % 60);
    format!("{y:04}{m:02}{d:02}T{h:02}{mi:02}{s:02}Z")
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
