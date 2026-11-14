use std::collections::HashSet;
use std::time::Duration;

use tempfile::TempDir;
use vesper_protocol::Role;
use vesper_server::auth::hash_token;
use vesper_server::cli;
use vesper_server::config::Config;
use vesper_store::{NewAccount, NewSession, Store, now_ms};

fn config(data: &TempDir) -> Config {
    Config {
        http_bind: "127.0.0.1:0".parse().unwrap(),
        udp_bind: "127.0.0.1:0".parse().unwrap(),
        public_host: None,
        udp_port_public: 0,
        max_room_size: 12,
        ring_timeout: std::time::Duration::from_secs(30),
        ping_interval: Duration::from_secs(30),
        max_sockets: 4,
        web_dir: "does-not-exist".into(),
        data_dir: data.path().to_path_buf(),
        origin: "http://localhost:8080".into(),
        secure_cookies: false,
        upload_quota_bytes: 0,
        upload_max_bytes: 100 << 20,
        push_subject: None,
        build: "test-build".into(),
        backup_keep: 5,
        backup_every_hours: 0,
        backup_uploads: false,
        header_timeout: vesper_server::config::HEADER_TIMEOUT,
        body_idle: vesper_server::config::BODY_IDLE,
        body_total: vesper_server::config::BODY_TOTAL,
    }
}

async fn run(cfg: &Config, args: &[&str]) -> Result<Option<String>, String> {
    let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    cli::run(&args, cfg).await
}

async fn store(data: &TempDir) -> Store {
    Store::open(&data.path().join("vesper.db")).await.unwrap()
}

/// The code after `#` in a printed link.
fn code(link: &str) -> &str {
    let code = link
        .strip_prefix("http://localhost:8080/invite#")
        .unwrap_or_else(|| panic!("not an invite link: {link}"));
    assert_eq!(code.len(), 64, "{code}");
    assert!(
        code.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    );
    code
}

#[tokio::test]
async fn no_subcommand_means_serve() {
    let data = TempDir::new().unwrap();
    assert_eq!(run(&config(&data), &[]).await, Ok(None));
    assert!(run(&config(&data), &["nope"]).await.is_err());
    assert!(
        run(&config(&data), &["invite", "--role", "owner"])
            .await
            .is_err()
    );
    assert!(
        run(&config(&data), &["invite", "--uses", "0"])
            .await
            .is_err()
    );
    let help = run(&config(&data), &["help"]).await.unwrap().unwrap();
    assert!(help.contains("Throwaway data dir only"), "{help}");
}

#[tokio::test]
async fn create_owner_prints_link_once() {
    let data = TempDir::new().unwrap();
    let cfg = config(&data);
    let first = run(&cfg, &["create-owner"]).await.unwrap().unwrap();
    // Nobody signed up yet: a second run still works, in case the first link was lost.
    let second = run(&cfg, &["create-owner"]).await.unwrap().unwrap();
    assert_ne!(first, second);
    let store = store(&data).await;
    let hash = hash_token(code(&second)).unwrap();
    assert_eq!(
        store.invite_role(&hash, now_ms()).await.unwrap(),
        Some(Role::Owner)
    );
    let (user, _) = store
        .signup(
            &hash,
            NewAccount {
                username: "toby",
                display_name: "Toby",
                password_phc: None,
                role: Role::Member,
            },
            NewSession {
                token_hash: [7; 32],
                label: "test",
                now: now_ms(),
            },
        )
        .await
        .unwrap();
    assert_eq!(user.role, Role::Owner);
    assert_eq!(
        run(&cfg, &["create-owner"]).await,
        Err("an owner already exists".into())
    );
}

#[tokio::test]
async fn invite_cli_defaults() {
    let data = TempDir::new().unwrap();
    let before = now_ms();
    let link = run(&config(&data), &["invite"]).await.unwrap().unwrap();
    code(&link);
    let invites = store(&data).await.invites().await.unwrap();
    assert_eq!(invites.len(), 1);
    let i = &invites[0];
    assert_eq!((i.role, i.uses, i.max_uses), (Role::Member, 0, 1));
    let week = 168 * 3600 * 1000;
    assert!(i.expires_at >= before + week && i.expires_at <= now_ms() + week);

    run(
        &config(&data),
        &[
            "invite",
            "--role",
            "moderator",
            "--uses",
            "3",
            "--hours",
            "2",
        ],
    )
    .await
    .unwrap();
    let i = &store(&data).await.invites().await.unwrap()[0];
    assert_eq!((i.role, i.max_uses), (Role::Moderator, 3));
}

#[tokio::test]
async fn loadtest_accounts_tokens_authenticate() {
    let data = TempDir::new().unwrap();
    let out = run(&config(&data), &["loadtest-accounts", "--count", "3"])
        .await
        .unwrap()
        .unwrap();
    let store = store(&data).await;
    let mut ids = HashSet::new();
    for token in out.lines() {
        let hash = hash_token(token).expect("64 hex");
        let (_, user) = store.session_user(&hash, now_ms()).await.unwrap().unwrap();
        assert_eq!(user.role, Role::Member);
        assert!(user.username.starts_with("load_"), "{}", user.username);
        ids.insert(user.user_id);
    }
    assert_eq!(ids.len(), 3);
    assert!(run(&config(&data), &["loadtest-accounts"]).await.is_err());
}

#[tokio::test]
async fn loadtest_accounts_refuses_a_used_data_dir() {
    let data = TempDir::new().unwrap();
    let cfg = config(&data);
    run(&cfg, &["loadtest-accounts", "--count", "1"])
        .await
        .unwrap();
    let e = run(&cfg, &["loadtest-accounts", "--count", "1"])
        .await
        .unwrap_err();
    assert!(e.contains("already has accounts"), "{e}");
    let forced = [
        "loadtest-accounts",
        "--count",
        "1",
        "--force-nonempty",
        "yes",
    ];
    assert!(run(&cfg, &forced).await.is_ok());
}

#[cfg(unix)]
#[tokio::test]
async fn new_data_dir_and_db_files_are_private() {
    use std::os::unix::fs::PermissionsExt;
    let root = TempDir::new().unwrap();
    let dir = root.path().join("a/data");
    let store = Store::open(&dir.join("vesper.db")).await.unwrap();
    store.owner_exists().await.unwrap();
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&dir), 0o700);
    for f in std::fs::read_dir(&dir).unwrap() {
        let f = f.unwrap().path();
        assert_eq!(mode(&f), 0o600, "{}", f.display());
    }
}

#[test]
fn owner_link_never_reaches_the_logs() {
    let data = TempDir::new().unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_vesper-server"))
        .arg("create-owner")
        .env("RUST_LOG", "trace")
        .env("VESPER_DATA_DIR", data.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8(out.stdout).unwrap();
    let stderr = String::from_utf8(out.stderr).unwrap();
    let c = code(stdout.trim());
    assert!(!stderr.is_empty(), "trace logging should have run");
    assert!(
        !stderr.contains(c) && !stderr.contains("/invite#"),
        "{stderr}"
    );
}

#[test]
fn utc_stamp_is_the_calendar_time() {
    assert_eq!(cli::utc_stamp(0), "19700101T000000Z");
    // 2024-02-29 23:59:59 UTC, a leap day.
    assert_eq!(cli::utc_stamp(1_709_251_199_000), "20240229T235959Z");
    assert_eq!(cli::utc_stamp(1_709_251_200_000), "20240301T000000Z");
}

#[cfg(unix)]
#[tokio::test]
async fn backup_copies_db_and_uploads_privately_and_never_overwrites() {
    use std::os::unix::fs::PermissionsExt;
    let data = TempDir::new().unwrap();
    let cfg = config(&data);
    let s = store(&data).await;
    s.create_room("games", 1).await.unwrap();
    let shard = data.path().join("uploads/ab");
    std::fs::create_dir_all(&shard).unwrap();
    std::fs::write(shard.join("ab00"), b"hello").unwrap();
    std::fs::create_dir_all(data.path().join("uploads/tmp")).unwrap();
    std::fs::write(data.path().join("uploads/tmp/partial"), b"x").unwrap();

    let out_root = TempDir::new().unwrap();
    let dir = out_root.path().join("backups");
    let now = 1_709_251_199_000;
    let summary = cli::backup(&cfg, &dir, now).await.unwrap();
    assert!(summary.contains("1 files, 5 bytes"), "{summary}");
    let db = dir.join("vesper-20240229T235959Z.db");
    let up = dir.join("vesper-20240229T235959Z-uploads");
    let copy = Store::open(&db).await.unwrap();
    assert_eq!(copy.rooms().await.unwrap(), s.rooms().await.unwrap());
    assert_eq!(std::fs::read(up.join("ab/ab00")).unwrap(), b"hello");
    assert!(!up.join("tmp").exists());

    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    for d in [&dir, &up, &up.join("ab")] {
        assert_eq!(mode(d), 0o700, "{}", d.display());
    }
    for f in [&db, &up.join("ab/ab00")] {
        assert_eq!(mode(f), 0o600, "{}", f.display());
    }

    std::fs::write(shard.join("ab00"), b"changed").unwrap();
    let e = cli::backup(&cfg, &dir, now).await.unwrap_err();
    assert!(e.contains("not overwriting"), "{e}");
    assert_eq!(std::fs::read(up.join("ab/ab00")).unwrap(), b"hello");
    // The uploads dir alone also blocks it.
    std::fs::remove_file(&db).unwrap();
    assert!(cli::backup(&cfg, &dir, now).await.is_err());
    assert!(!db.exists());

    assert!(run(&cfg, &["backup"]).await.is_err());
    assert!(run(&cfg, &["backup", "a", "b"]).await.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn failed_backup_leaves_nothing_behind() {
    use std::os::unix::fs::PermissionsExt;
    let data = TempDir::new().unwrap();
    let cfg = config(&data);
    store(&data).await;
    let shard = data.path().join("uploads/ab");
    std::fs::create_dir_all(&shard).unwrap();
    std::fs::write(shard.join("ab01"), b"ok").unwrap();
    let locked = shard.join("ab02");
    std::fs::write(&locked, b"secret").unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    if std::fs::File::open(&locked).is_ok() {
        return; // root reads anything: this failure can not be forced here
    }
    let out = TempDir::new().unwrap();
    let e = cli::backup(&cfg, out.path(), 0).await.unwrap_err();
    assert!(e.contains("partial backup was removed"), "{e}");
    let left: Vec<_> = std::fs::read_dir(out.path()).unwrap().collect();
    assert!(left.is_empty(), "{left:?}");
}

#[tokio::test]
async fn backup_refuses_a_data_dir_without_a_database() {
    let data = TempDir::new().unwrap();
    let out = TempDir::new().unwrap();
    let e = cli::backup(&config(&data), out.path(), 0)
        .await
        .unwrap_err();
    assert!(e.contains("not found"), "{e}");
    assert!(!data.path().join("vesper.db").exists());
    assert_eq!(std::fs::read_dir(out.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn backup_dir_inside_uploads_is_refused() {
    let data = TempDir::new().unwrap();
    store(&data).await;
    std::fs::create_dir_all(data.path().join("uploads")).unwrap();
    let e = cli::backup(&config(&data), &data.path().join("uploads/backups"), 0)
        .await
        .unwrap_err();
    assert!(e.contains("inside"), "{e}");
}

fn names(dir: &std::path::Path) -> Vec<String> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    v.sort();
    v
}

fn sfu() -> vesper_sfu::SfuHandle {
    vesper_sfu::Sfu::spawn(vesper_sfu::SfuConfig {
        bind: "127.0.0.1:0".parse().unwrap(),
        ..Default::default()
    })
    .unwrap()
}

/// An old-schema database (the first 8 migrations, v1.2's schema plus 0007 and 0008): the start
/// writes `pre-schema-0008-<t>.db` and its manifest before it migrates, and the copy is
/// still at schema 8.
#[cfg(unix)]
#[tokio::test]
async fn start_backs_up_an_old_database_before_migrating() {
    use std::os::unix::fs::PermissionsExt;
    let data = TempDir::new().unwrap();
    let db = data.path().join("vesper.db");
    Store::create_at_schema(&db, 8).await.unwrap();
    let (_, newest) = Store::schema_versions(&db).await.unwrap().unwrap();
    std::fs::create_dir_all(data.path().join("uploads/ab")).unwrap();
    std::fs::write(data.path().join("uploads/ab/ab00"), b"big file").unwrap();

    let (_addr, _serving) = vesper_server::start_with_sfu(config(&data), sfu())
        .await
        .unwrap();
    assert_eq!(
        Store::schema_versions(&db).await.unwrap(),
        Some((newest, newest))
    );
    let dir = data.path().join("backups");
    let files = names(&dir);
    assert_eq!(files.len(), 2, "{files:?}");
    let stem = files[0].strip_suffix(".db").unwrap();
    assert!(stem.starts_with("pre-schema-0008-"), "{stem}");
    assert_eq!(files[1], format!("{stem}.json"));
    let copy = dir.join(&files[0]);
    assert_eq!(
        Store::schema_versions(&copy).await.unwrap(),
        Some((8, newest))
    );
    let man: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join(&files[1])).unwrap()).unwrap();
    assert_eq!(man["kind"], "pre-migration");
    assert_eq!(man["schema"], 8);
    assert_eq!(man["schema_to"], newest);
    assert_eq!(man["server_build"], "test-build");
    assert_eq!(man["uploads"], false);
    let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&dir), 0o700);
    assert_eq!(mode(&copy), 0o600);
    assert_eq!(mode(&dir.join(&files[1])), 0o600);
    // The copy opens on this code and holds the rooms.
    let rooms = Store::open(&copy).await.unwrap().rooms().await.unwrap();
    assert!(rooms.iter().any(|r| r.name == "general"), "{rooms:?}");

    // Nothing pending now: a second start takes no backup.
    let (_addr, _serving) = vesper_server::start_with_sfu(config(&data), sfu())
        .await
        .unwrap();
    // Only `.db` names: the copy opened above may still have its -wal and -shm next to it.
    let dbs = names(&dir)
        .into_iter()
        .filter(|n| n.ends_with(".db"))
        .count();
    assert_eq!(dbs, 1, "no new backup");
}

/// A new data dir has nothing to back up; a failed backup stops the start before any migration.
#[tokio::test]
async fn failed_backup_before_migration_stops_the_start() {
    let data = TempDir::new().unwrap();
    let (_addr, _serving) = vesper_server::start_with_sfu(config(&data), sfu())
        .await
        .unwrap();
    assert!(!data.path().join("backups").exists());

    let data = TempDir::new().unwrap();
    let db = data.path().join("vesper.db");
    Store::create_at_schema(&db, 8).await.unwrap();
    // A file where the backups dir goes: the backup can not be written.
    std::fs::write(data.path().join("backups"), b"").unwrap();
    let e = vesper_server::start_with_sfu(config(&data), sfu())
        .await
        .unwrap_err()
        .to_string();
    assert!(e.contains("does not start"), "{e}");
    assert_eq!(Store::schema_versions(&db).await.unwrap().unwrap().0, 8);
}

/// Each kind keeps its newest `backup_keep`; uploads only with `backup_uploads`; manual
/// `vesper-*` backups are never deleted.
#[tokio::test]
async fn automatic_backups_keep_the_newest_of_each_kind() {
    let data = TempDir::new().unwrap();
    let mut cfg = config(&data);
    cfg.backup_keep = 2;
    store(&data).await;
    let dir = data.path().join("backups");
    cli::backup(&cfg, &dir, 0).await.unwrap();
    let pre = cli::Auto::PreMigration { from: 9, to: 9 };
    cli::auto_backup(&cfg, pre, 1_000).await.unwrap();
    for t in [5_000, 3_000, 4_000, 2_000] {
        cli::auto_backup(&cfg, cli::Auto::Scheduled, t)
            .await
            .unwrap();
    }
    cfg.backup_uploads = true;
    cli::auto_backup(&cfg, cli::Auto::Scheduled, 6_000)
        .await
        .unwrap();
    assert_eq!(
        names(&dir),
        [
            "auto-19700101T000005.000Z.db",
            "auto-19700101T000005.000Z.json",
            "auto-19700101T000006.000Z-uploads",
            "auto-19700101T000006.000Z.db",
            "auto-19700101T000006.000Z.json",
            "pre-schema-0009-19700101T000001.000Z.db",
            "pre-schema-0009-19700101T000001.000Z.json",
            "vesper-19700101T000000Z-uploads",
            "vesper-19700101T000000Z.db",
        ]
    );
}

/// The backups whose name says they hold a schema an image of `old_schema` supports, newest
/// first: the restore recipe of docs/deploy.md ("Upgrade and rollback") as code.
fn restore_pick(dir: &std::path::Path, old_schema: i64) -> std::path::PathBuf {
    let mut ok: Vec<(String, String)> = names(dir)
        .into_iter()
        .filter_map(|n| {
            let rest = n.strip_prefix("pre-schema-")?.strip_suffix(".db")?;
            let (schema, stamp) = rest.split_once('-')?;
            (schema.parse::<i64>().ok()? <= old_schema).then(|| (stamp.to_owned(), n.clone()))
        })
        .collect();
    ok.sort();
    dir.join(&ok.last().expect("no backup the old image can open").1)
}

/// A release moves the schema from 7 to 9 and 9 fails on the data: start 1 commits 8, then
/// every restart (compose `restart: unless-stopped`) backs up the schema-8 state and fails
/// again. Five starts with `backup_keep` 2: the schema-7 copy is kept, and the copy the restore
/// recipe picks for the old image (schema 7) opens with the old migration set.
#[tokio::test]
async fn crash_loop_after_a_partial_migration_keeps_the_old_schema_copy() {
    let data = TempDir::new().unwrap();
    let db = data.path().join("vesper.db");
    Store::create_at_schema(&db, 7).await.unwrap();
    let mut cfg = config(&data);
    cfg.backup_keep = 2;
    for _ in 0..5 {
        vesper_server::backup_before_migration(&cfg, &db)
            .await
            .unwrap();
        Store::migrate_failing_at(&db, 9).await.unwrap_err();
        tokio::time::sleep(Duration::from_millis(2)).await; // a new stamp
    }
    assert_eq!(Store::schema_versions(&db).await.unwrap().unwrap().0, 8);
    let dir = data.path().join("backups");
    let schemas: Vec<String> = names(&dir)
        .into_iter()
        .filter(|n| n.ends_with(".db"))
        .map(|n| n[..15].to_owned())
        .collect();
    assert_eq!(
        schemas,
        ["pre-schema-0007", "pre-schema-0008", "pre-schema-0008"]
    );
    let pick = restore_pick(&dir, 7);
    assert!(
        pick.to_string_lossy().contains("pre-schema-0007-"),
        "{pick:?}"
    );
    // The old image's migrations (the first 7) accept the copy and change nothing.
    Store::create_at_schema(&pick, 7).await.unwrap();
    assert_eq!(Store::schema_versions(&pick).await.unwrap().unwrap().0, 7);
    // And refuse the half-migrated database itself: the copy is needed.
    Store::create_at_schema(&db, 7).await.unwrap_err();
}

/// A data dir with `count` small upload files, for the slow backups below.
fn many_uploads(data: &TempDir, count: usize) {
    for i in 0..count {
        let shard = data.path().join(format!("uploads/{:02x}", i % 256));
        std::fs::create_dir_all(&shard).unwrap();
        std::fs::write(shard.join(format!("f{i}")), [7u8; 4096]).unwrap();
    }
}

/// A backup killed while it is written (here: its task aborted while the uploads copy runs)
/// leaves only `.partial` names, never a `pre-*.db`, `.json` or `-uploads` that looks complete;
/// the next automatic backup deletes the leftovers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_killed_backup_leaves_no_complete_looking_file() {
    let data = TempDir::new().unwrap();
    store(&data).await;
    many_uploads(&data, 4000);
    let mut cfg = config(&data);
    cfg.backup_uploads = true;
    let dir = data.path().join("backups");
    let pre = cli::Auto::PreMigration { from: 9, to: 9 };
    let c = cfg.clone();
    let task = tokio::spawn(async move { cli::auto_backup(&c, pre, 1_000).await });
    let started = std::time::Instant::now();
    while !names_or_empty(&dir).iter().any(|n| n.contains("-uploads")) {
        assert!(
            started.elapsed() < Duration::from_secs(10),
            "{:?}",
            names_or_empty(&dir)
        );
        tokio::time::sleep(Duration::from_micros(200)).await;
    }
    task.abort();
    let _ = task.await;
    let left = names(&dir);
    assert!(left.iter().all(|n| n.ends_with(".partial")), "{left:?}");
    // The aborted task's blocking copy may still run; let it end, as a killed process would.
    // Wait until every file is copied (a fixed sleep was too short under CPU load).
    let upp = dir.join(left.iter().find(|n| n.contains("-uploads")).unwrap());
    let started = std::time::Instant::now();
    while count_files(&upp) < 4000 {
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "copy did not finish: {} files",
            count_files(&upp)
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(names(&dir).iter().all(|n| n.ends_with(".partial")));

    cfg.backup_uploads = false;
    cli::auto_backup(&cfg, pre, 2_000).await.unwrap();
    assert_eq!(
        names(&dir),
        [
            "pre-schema-0009-19700101T000002.000Z.db",
            "pre-schema-0009-19700101T000002.000Z.json"
        ]
    );
}

/// Regular files under `dir`, at any depth.
fn count_files(dir: &std::path::Path) -> usize {
    std::fs::read_dir(dir).map_or(0, |rd| {
        rd.flatten()
            .map(|e| match e.file_type() {
                Ok(t) if t.is_dir() => count_files(&e.path()),
                Ok(t) if t.is_file() => 1,
                _ => 0,
            })
            .sum()
    })
}

fn names_or_empty(dir: &std::path::Path) -> Vec<String> {
    if dir.exists() { names(dir) } else { Vec::new() }
}

/// The uploads copy and the prune run off the async workers: on a one-thread runtime a ping
/// task keeps running during a slow backup (4000 files).
#[tokio::test(flavor = "current_thread")]
async fn a_slow_backup_does_not_block_the_runtime() {
    let data = TempDir::new().unwrap();
    store(&data).await;
    many_uploads(&data, 4000);
    let mut cfg = config(&data);
    cfg.backup_uploads = true;
    let gap = std::sync::Arc::new(std::sync::Mutex::new(Duration::ZERO));
    let g = gap.clone();
    let ping = tokio::spawn(async move {
        let mut last = std::time::Instant::now();
        loop {
            tokio::time::sleep(Duration::from_millis(1)).await;
            let mut g = g.lock().unwrap();
            *g = (*g).max(last.elapsed());
            last = std::time::Instant::now();
        }
    });
    let t = std::time::Instant::now();
    cli::auto_backup(&cfg, cli::Auto::Scheduled, 1_000)
        .await
        .unwrap();
    let took = t.elapsed();
    ping.abort();
    let gap = *gap.lock().unwrap();
    assert!(
        took > Duration::from_millis(30),
        "too fast to tell: {took:?}"
    );
    assert!(gap * 3 < took, "the runtime stalled {gap:?} of {took:?}");
}

/// The account commands never migrate an existing database (that would skip the backup the
/// server takes first); they say to start the server once. A new data dir still works.
#[tokio::test]
async fn account_commands_refuse_a_database_with_pending_migrations() {
    let data = TempDir::new().unwrap();
    let db = data.path().join("vesper.db");
    Store::create_at_schema(&db, 8).await.unwrap();
    let cfg = config(&data);
    for args in [
        &["create-owner"][..],
        &["invite"],
        &["reset-password", "alice"],
        &["loadtest-accounts", "--count", "1"],
    ] {
        let e = run(&cfg, args).await.unwrap_err();
        assert!(e.contains("Start the server once first"), "{args:?}: {e}");
    }
    assert_eq!(Store::schema_versions(&db).await.unwrap().unwrap().0, 8);
    assert!(!data.path().join("backups").exists());
    // After the server migrated (here: `Store::open`), they work.
    drop(Store::open(&db).await.unwrap());
    run(&cfg, &["invite"]).await.unwrap();
}

#[test]
fn utc_stamp_ms_has_milliseconds() {
    assert_eq!(cli::utc_stamp_ms(1_234), "19700101T000001.234Z");
    assert_eq!(cli::utc_stamp_ms(-1), "19691231T235959.999Z");
}

/// Prune touches only names of the exact automatic shape: other files in `backups/`, a
/// non-ASCII one too, stay and do not panic it.
#[tokio::test]
async fn prune_leaves_other_names_alone() {
    let data = TempDir::new().unwrap();
    store(&data).await;
    let mut cfg = config(&data);
    cfg.backup_keep = 1;
    let dir = data.path().join("backups");
    std::fs::create_dir_all(&dir).unwrap();
    let junk = [
        "auto-notes.db",
        "pre-schema-0009-ü.db",
        "pre-schema-00x9-19700101T000001.000Z.db",
    ];
    for j in junk {
        std::fs::write(dir.join(j), b"").unwrap();
    }
    for t in [1_000, 2_000] {
        cli::auto_backup(&cfg, cli::Auto::Scheduled, t)
            .await
            .unwrap();
    }
    let left = names(&dir);
    for j in junk {
        assert!(left.contains(&j.to_owned()), "{left:?}");
    }
    assert!(
        !left.iter().any(|n| n.starts_with("auto-19700101T000001")),
        "{left:?}"
    );
    assert!(
        left.contains(&"auto-19700101T000002.000Z.db".to_owned()),
        "{left:?}"
    );
}

/// A clock that went back makes the copy just written look oldest: prune keeps it anyway,
/// because the server migrates right after it.
#[tokio::test]
async fn prune_never_deletes_the_copy_just_written() {
    let data = TempDir::new().unwrap();
    store(&data).await;
    let mut cfg = config(&data);
    cfg.backup_keep = 1;
    cli::auto_backup(&cfg, cli::Auto::Scheduled, 5_000)
        .await
        .unwrap();
    let p = cli::auto_backup(&cfg, cli::Auto::Scheduled, 1_000)
        .await
        .unwrap();
    assert!(p.is_file(), "{p:?}");
}

/// At start the leftovers of killed backups go: automatic `.json` and `-uploads/` without
/// their `.db`, and `vesper-*` `.partial` names older than an hour. Complete backups, a fresh
/// manual `.partial` (it may be running) and other names stay.
#[tokio::test]
async fn start_removes_orphans_of_killed_backups() {
    let data = TempDir::new().unwrap();
    store(&data).await;
    let cfg = config(&data);
    let dir = data.path().join("backups");
    cli::auto_backup(&cfg, cli::Auto::Scheduled, 1_000)
        .await
        .unwrap();
    cli::backup(&cfg, &dir, 0).await.unwrap();
    let orphan = "pre-schema-0009-19700101T000002.000Z";
    std::fs::write(dir.join(format!("{orphan}.json")), b"{}").unwrap();
    std::fs::create_dir_all(dir.join(format!("{orphan}-uploads/ab"))).unwrap();
    let old = dir.join("vesper-19700101T000003Z.db.partial");
    let f = std::fs::File::create(&old).unwrap();
    f.set_modified(std::time::SystemTime::now() - Duration::from_secs(7200))
        .unwrap();
    std::fs::write(dir.join("vesper-19700101T000004Z.db.partial"), b"").unwrap();
    std::fs::write(dir.join("notes.json"), b"").unwrap();
    cli::clean_backups_dir(&cfg).await;
    assert_eq!(
        names(&dir),
        [
            "auto-19700101T000001.000Z.db",
            "auto-19700101T000001.000Z.json",
            "notes.json",
            "vesper-19700101T000000Z-uploads",
            "vesper-19700101T000000Z.db",
            "vesper-19700101T000004Z.db.partial",
        ]
    );
}

/// With `VESPER_BACKUP_UPLOADS=1`, uploads that would leave less free disk than twice their
/// size are not copied (the database still is), and the manifest says so.
#[tokio::test]
async fn automatic_backup_skips_uploads_that_would_fill_the_disk() {
    let data = TempDir::new().unwrap();
    let s = store(&data).await;
    let user = s
        .create_account(NewAccount {
            username: "alice_1",
            display_name: "alice_1",
            password_phc: None,
            role: Role::Member,
        })
        .await
        .unwrap();
    let room = s.rooms().await.unwrap()[0].id;
    s.insert_upload(&vesper_store::UploadRow {
        id: "ab".repeat(16),
        room_id: room,
        uploader_id: user.user_id.clone(),
        message_id: None,
        name: "big.bin".into(),
        mime: "application/octet-stream".into(),
        size: i64::MAX / 4,
        created_at: 0,
    })
    .await
    .unwrap();
    std::fs::create_dir_all(data.path().join("uploads/ab")).unwrap();
    std::fs::write(data.path().join("uploads/ab/x"), b"x").unwrap();
    let mut cfg = config(&data);
    cfg.backup_uploads = true;
    let p = cli::auto_backup(&cfg, cli::Auto::Scheduled, 1_000)
        .await
        .unwrap();
    let dir = data.path().join("backups");
    assert_eq!(
        names(&dir),
        [
            "auto-19700101T000001.000Z.db",
            "auto-19700101T000001.000Z.json"
        ]
    );
    let man: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(p.with_extension("json")).unwrap()).unwrap();
    assert_eq!(man["uploads"], false);
}
