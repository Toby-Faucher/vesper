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
/// into `dir` (made 0700). Each is written under its name plus `.partial` and renamed into place
/// only when all are written, the database last: a `<stem>.db` on disk is complete after a kill,
/// and a `.partial` never is. The database, the manifest and the directory are synced, so the
/// database copy also survives a power cut; the copied upload files are not synced (one fsync per
/// file is slow), so after a power cut some of them can be empty. The database copy comes first,
/// then the uploads: a file uploaded meanwhile is an extra file the restored server's sweep
/// removes, and a file deleted meanwhile is counted as skipped. On an error every output is removed.
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
    let part = |p: &Path| PathBuf::from(format!("{}{PARTIAL}", p.display()));
    let (dbp, upp, manp) = (part(&db), part(&up), part(&man));
    let all = [&db, &up, &man, &dbp, &upp, &manp];
    for p in all {
        if p.symlink_metadata().is_ok() {
            return Err(format!("{} exists; not overwriting", p.display()));
        }
    }
    // Created empty with mode 600 first: VACUUM INTO writes into an empty file and keeps its mode.
    private_file(&dbp).map_err(io(&dbp))?;
    let written = async {
        store
            .backup_into(&dbp)
            .await
            .map_err(|e| format!("{}: {e}", dbp.display()))?;
        let mut copied = None;
        if uploads {
            private_dir(&upp, false).map_err(io(&upp))?;
            let (s, d) = (src.clone(), upp.clone());
            // Up to the upload quota of plain file I/O: off the async workers.
            let n = tokio::task::spawn_blocking(move || {
                let mut n = Copied::default();
                copy_uploads(&s, &d, &mut n).map(|()| n)
            })
            .await
            .map_err(|e| format!("{}: {e}", upp.display()))?
            .map_err(io(&upp))?;
            copied = Some((up.clone(), n));
        }
        let (dir, db, up, man, dbp, upp, manp, last) = (
            dir.to_path_buf(),
            db.clone(),
            up.clone(),
            man.clone(),
            dbp.clone(),
            upp.clone(),
            manp.clone(),
            db.clone(),
        );
        // fsyncs can take a while on a slow disk: off the async workers.
        let db_bytes = tokio::task::spawn_blocking(move || {
            if let Some(text) = &manifest {
                private_file(&manp)
                    .and_then(|mut f| {
                        std::io::Write::write_all(&mut f, text.as_bytes())?;
                        f.sync_all()
                    })
                    .map_err(io(&manp))?;
            }
            std::fs::File::open(&dbp)
                .and_then(|f| f.sync_all())
                .map_err(io(&dbp))?;
            if uploads {
                std::fs::rename(&upp, &up).map_err(io(&up))?;
            }
            if manifest.is_some() {
                std::fs::rename(&manp, &man).map_err(io(&man))?;
            }
            std::fs::rename(&dbp, &db).map_err(io(&db))?;
            // The renames themselves, so the names survive a power cut.
            std::fs::File::open(&dir)
                .and_then(|f| f.sync_all())
                .map_err(io(&dir))?;
            std::fs::metadata(&db).map(|m| m.len()).map_err(io(&db))
        })
        .await
        .map_err(|e| format!("{}: {e}", last.display()))??;
        Ok::<_, String>(Written {
            db: last,
            db_bytes,
            uploads: copied,
        })
    }
    .await;
    drop(store);
    written.map_err(|e| {
        for p in all {
            let _ = std::fs::remove_file(p).or_else(|_| std::fs::remove_dir_all(p));
        }
        format!("{e}; the partial backup was removed")
    })
}

/// The suffix of a backup file still being written. Never a complete backup.
const PARTIAL: &str = ".partial";

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
            Auto::PreMigration { .. } => "pre-schema-",
            Auto::Scheduled => "auto-",
        }
    }
}

/// An automatic backup into `<data>/backups`: `pre-schema-<schema>-<UTC time>` (the schema
/// number is the one the copy holds, so an older image that knows it opens the copy) or
/// `auto-<UTC time>`, the database and a small JSON manifest, plus the uploads only with
/// `VESPER_BACKUP_UPLOADS=1` (they can be large). First, `.partial` leftovers of an automatic
/// backup that was killed are deleted; after, [`prune`]. Returns the database copy's path.
pub async fn auto_backup(cfg: &Config, kind: Auto, now: i64) -> Result<PathBuf, String> {
    let dir = cfg.data_dir.join("backups");
    let stamp = utc_stamp_ms(now);
    let stem = format!(
        "{}{}",
        kind.prefix(),
        match kind {
            Auto::PreMigration { from, .. } => format!("{from:04}-{stamp}"),
            Auto::Scheduled => stamp.clone(),
        }
    );
    let uploads = cfg.backup_uploads && uploads_fit(cfg).await;
    let (schema, to) = match kind {
        Auto::PreMigration { from, to } => (Some(from), Some(to)),
        Auto::Scheduled => (None, None),
    };
    let manifest = serde_json::json!({
        "kind": match kind { Auto::PreMigration { .. } => "pre-migration", Auto::Scheduled => "scheduled" },
        "time": stamp,
        "server_version": crate::VERSION,
        "server_build": cfg.build,
        "schema": schema,
        "schema_to": to,
        "uploads": uploads,
    });
    clean_backups_dir(cfg).await;
    let w = write_backup(cfg, &dir, &stem, uploads, Some(format!("{manifest:#}\n"))).await?;
    let (d, keep) = (dir.clone(), cfg.backup_keep);
    // The backup is written: a failed prune must not stop the start.
    blocking(move || prune(&d, kind.prefix(), keep, &stem)).await;
    Ok(w.db)
}

/// False, with the reason logged, when copying `uploads/` would leave less free disk than twice
/// its size: a crash loop with `VESPER_BACKUP_UPLOADS=1` copies it at every start. The database
/// is still copied.
async fn uploads_fit(cfg: &Config) -> bool {
    let size = match Store::open_existing(&cfg.data_dir.join("vesper.db")).await {
        Ok(s) => s.upload_bytes().await.map(|n| n.max(0) as u64),
        Err(e) => Err(e),
    };
    let free = rustix::fs::statvfs(cfg.data_dir.as_path())
        .map(|v| v.f_bavail.saturating_mul(v.f_frsize))
        .map_err(std::io::Error::from);
    match (size, free) {
        (Ok(size), Ok(free)) if free.saturating_sub(size) >= size.saturating_mul(2) => true,
        (Ok(size), Ok(free)) => {
            tracing::warn!(
                "automatic backup without uploads: copying {size} bytes would leave {} of {free} free bytes, under twice the uploads",
                free.saturating_sub(size)
            );
            false
        }
        (size, free) => {
            tracing::warn!(
                "automatic backup without uploads: could not measure them ({:?}, {:?})",
                size.err(),
                free.err()
            );
            false
        }
    }
}

/// Runs [`remove_leftovers`] on `<data>/backups`, at every start and before each automatic
/// backup. Errors are logged.
pub async fn clean_backups_dir(cfg: &Config) {
    let d = cfg.data_dir.join("backups");
    blocking(move || remove_leftovers(&d, std::time::SystemTime::now())).await;
}

/// `f` on a blocking thread; an error is logged, not returned.
async fn blocking(f: impl FnOnce() -> std::io::Result<()> + Send + 'static) {
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => tracing::warn!("backups dir: {e}"),
        Err(e) => tracing::warn!("backups dir: {e}"),
    }
}

/// Deletes what killed backups leave behind: `pre-*` and `auto-*` names ending in `.partial`,
/// a `pre-schema-*` or `auto-*` `.json` or `-uploads/` whose `.db` is missing (a kill between
/// the renames), and `vesper-*` `.partial` names older than an hour (a manual backup may be
/// running now; none takes that long on this data). Complete manual `vesper-*` backups and any
/// other name are never touched. Automatic backups run one at a time, so none is being written.
fn remove_leftovers(dir: &Path, now: std::time::SystemTime) -> std::io::Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(d) => d,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let auto_db = |stem: &str| {
        let n = format!("{stem}.db");
        (parse_auto(&n, "pre-schema-").is_some() || parse_auto(&n, "auto-").is_some()).then_some(n)
    };
    for e in entries {
        let e = e?;
        let name = e.file_name();
        let name = name.to_string_lossy();
        let partial = name.ends_with(PARTIAL);
        let old = || {
            e.metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|t| now.duration_since(t).is_ok_and(|d| d.as_secs() > 3600))
        };
        let orphan = || {
            let stem = name
                .strip_suffix(".json")
                .or_else(|| name.strip_suffix("-uploads"));
            stem.and_then(auto_db)
                .is_some_and(|db| !dir.join(db).exists())
        };
        let gone = if partial {
            name.starts_with("pre-")
                || name.starts_with("auto-")
                || (name.starts_with("vesper-") && old())
        } else {
            orphan()
        };
        if gone {
            let p = e.path();
            if e.file_type()?.is_dir() {
                std::fs::remove_dir_all(&p)?;
            } else {
                std::fs::remove_file(&p)?;
            }
            tracing::warn!(
                "removed {}, left by a backup that did not finish",
                p.display()
            );
        }
    }
    Ok(())
}

/// `(schema, UTC time)` of an automatic backup's database file named `<prefix>`, then for
/// `pre-schema-` four digits and `-`, then a time like `20261101T203000.123Z`, then `.db`.
/// Anything else, a `.partial` too, is not one and is never deleted.
fn parse_auto<'a>(name: &'a str, prefix: &str) -> Option<(i64, &'a str)> {
    let rest = name.strip_suffix(".db")?.strip_prefix(prefix)?;
    let (schema, stamp) = if prefix == "pre-schema-" {
        let (s, t) = rest.split_once('-')?;
        (s.len() == 4 && s.bytes().all(|b| b.is_ascii_digit())).then_some(())?;
        (s.parse().ok()?, t)
    } else {
        (0, rest)
    };
    let shape = stamp.len() == 20
        && stamp.bytes().enumerate().all(|(i, b)| match i {
            8 => b == b'T',
            15 => b == b'.',
            19 => b == b'Z',
            _ => b.is_ascii_digit(),
        });
    shape.then_some((schema, stamp))
}

/// Deletes the backups of one kind (`prefix`) except the newest `keep` and, for each schema
/// number, the newest one of it: their `.db`, `.json` and `-uploads/`. The per-schema rule keeps
/// the copy an older image can open through a crash loop after an upgrade that failed part way
/// (each restart backs up the half-migrated schema; those never push out the older one).
/// `current`, the stem just written, is never deleted, even when a clock that went back makes it
/// look old: the server migrates right after it.
fn prune(dir: &Path, prefix: &str, keep: usize, current: &str) -> std::io::Result<()> {
    let names: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .collect();
    let mut found: Vec<(i64, &str, &str)> = names
        .iter()
        .filter_map(|n| {
            let (schema, stamp) = parse_auto(n, prefix)?;
            Some((schema, stamp, n.strip_suffix(".db")?))
        })
        .collect();
    found.sort_by(|a, b| b.1.cmp(a.1)); // newest first
    let mut seen = std::collections::HashSet::new();
    for (i, &(schema, _, stem)) in found.iter().enumerate() {
        let newest_of_schema = seen.insert(schema);
        if i < keep || newest_of_schema || stem == current {
            continue;
        }
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

/// `20261101T203000.123Z`: with milliseconds, so two starts in one second do not collide.
pub fn utc_stamp_ms(ms: i64) -> String {
    format!("{}.{:03}Z", &utc_stamp(ms)[..15], ms.rem_euclid(1000))
}

/// The database for the account commands. They never migrate an existing database: that
/// would skip the backup the server takes before it migrates. A new or empty one has nothing to
/// lose, so it is created and migrated as before.
async fn open(cfg: &Config) -> Result<Store, String> {
    let path = cfg.data_dir.join("vesper.db");
    let versions = Store::schema_versions(&path)
        .await
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    if let Some((applied, to)) = versions
        && applied > 0
        && applied < to
    {
        return Err(format!(
            "{} is at schema {applied} and this binary needs {to}; this command does not migrate. \
             Start the server once first (it backs the database up into {}/backups, then migrates), \
             then run this again",
            path.display(),
            cfg.data_dir.display()
        ));
    }
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
