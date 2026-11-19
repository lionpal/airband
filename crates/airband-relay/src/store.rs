//! Where the relay remembers which channels exist: the channel's name and the hash of its tower's pull token.
//! That is all it ever stores about anyone's traffic. It also keeps the homepage's waiting list: email addresses
//! people gave to be told when the app is ready, and nothing else about them. A folder (one JSON file) when run on a machine with a disk; a Postgres database
//! when run somewhere without one, such as a container.
//!
//! Postgres is chosen by setting DB_HOST. The relay keeps its own database on that server (DB_NAME, default
//! `airband`) and creates it and its one table on first start, so it can share a server with other applications
//! without touching anything of theirs.
//!
//!   DB_HOST, DB_PORT (5432), DB_USER, DB_PASS, DB_NAME (airband)
//!   DB_CA    a PEM file of certificate authorities to trust for the connection (for Amazon RDS, its global bundle)
//!   DB_SSL   `off` for a plain local database
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio_postgres::{Client, Config, NoTls};

pub enum Store {
    Folder(PathBuf),
    Postgres(Pg),
}

pub struct Pg {
    host: String,
    port: u16,
    user: String,
    pass: String,
    name: String,
    tls: Option<Arc<rustls::ClientConfig>>,
}

fn var(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

impl Store {
    /// From the environment. Fails at once, with the reason, on a half-configured database.
    pub fn from_env() -> Result<Store, String> {
        let Some(host) = var("DB_HOST") else {
            let dir = PathBuf::from(var("AIRBAND_RELAY_DATA").unwrap_or_else(|| "relay-data".into()));
            std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
            return Ok(Store::Folder(dir));
        };
        let tls = if var("DB_SSL").as_deref() == Some("off") {
            None
        } else {
            let ca = var("DB_CA").ok_or("DB_HOST is set, so set DB_CA to a PEM file of authorities to trust, or DB_SSL=off for a local database")?;
            let pem = std::fs::read(&ca).map_err(|e| format!("cannot read DB_CA {ca}: {e}"))?;
            let mut roots = rustls::RootCertStore::empty();
            for cert in rustls_pemfile::certs(&mut pem.as_slice()) {
                roots.add(cert.map_err(|e| format!("DB_CA {ca} is not PEM: {e}"))?).map_err(|e| format!("DB_CA {ca}: {e}"))?;
            }
            if roots.is_empty() {
                return Err(format!("DB_CA {ca} holds no certificates"));
            }
            let provider = Arc::new(rustls::crypto::ring::default_provider());
            let config = rustls::ClientConfig::builder_with_provider(provider).with_safe_default_protocol_versions().map_err(|e| e.to_string())?.with_root_certificates(roots).with_no_client_auth();
            Some(Arc::new(config))
        };
        Ok(Store::Postgres(Pg {
            host,
            port: var("DB_PORT").and_then(|p| p.parse().ok()).unwrap_or(5432),
            user: var("DB_USER").ok_or("DB_HOST is set but DB_USER is not")?,
            pass: var("DB_PASS").ok_or("DB_HOST is set but DB_PASS is not")?,
            name: var("DB_NAME").unwrap_or_else(|| "airband".into()),
            tls,
        }))
    }

    pub fn describe(&self) -> String {
        match self {
            Store::Folder(dir) => format!("the folder {}", dir.display()),
            Store::Postgres(pg) => format!("the database {} on {}", pg.name, pg.host),
        }
    }

    /// Every channel and its pull-token hash. For Postgres, also makes the database and table if they are missing.
    pub async fn load(&self) -> Result<HashMap<String, String>, String> {
        match self {
            Store::Folder(dir) => Ok(std::fs::read_to_string(dir.join("channels.json")).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()),
            Store::Postgres(pg) => {
                // a database cannot be created from inside itself: ask the server's own `postgres` database
                let admin = pg.connect("postgres").await?;
                let exists = admin.query_opt("select 1 from pg_database where datname = $1", &[&pg.name]).await.map_err(|e| e.to_string())?.is_some();
                if !exists {
                    if !pg.name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') {
                        return Err(format!("DB_NAME \"{}\" must be lowercase letters, digits and underscores", pg.name));
                    }
                    admin.batch_execute(&format!("create database {}", pg.name)).await.map_err(|e| format!("cannot create database {}: {e}", pg.name))?;
                }
                let db = pg.connect(&pg.name).await?;
                db.batch_execute(
                    "create table if not exists relay_channels (channel text primary key, pull_hash text not null, created_at timestamptz not null default now());
                     create table if not exists waitlist (email text primary key, joined_at timestamptz not null default now());
                     create table if not exists relay_held (seq bigserial primary key, channel text not null, id text not null, sealed bytea not null, held_at timestamptz not null default now(), unique (channel, id))",
                )
                .await
                .map_err(|e| e.to_string())?;
                let rows = db.query("select channel, pull_hash from relay_channels", &[]).await.map_err(|e| e.to_string())?;
                Ok(rows.iter().map(|r| (r.get(0), r.get(1))).collect())
            }
        }
    }

    /// Put an address on the waiting list. Returns only once it is kept; an address already there is left as it is.
    pub async fn join_waitlist(&self, email: &str) -> Result<(), String> {
        match self {
            Store::Folder(dir) => {
                let file = dir.join("waitlist.txt");
                let have = std::fs::read_to_string(&file).unwrap_or_default();
                if have.lines().any(|l| l == email) {
                    return Ok(());
                }
                std::fs::write(&file, format!("{have}{email}\n")).map_err(|e| format!("cannot save {}: {e}", file.display()))
            }
            Store::Postgres(pg) => {
                let db = pg.connect(&pg.name).await?;
                db.execute("insert into waitlist (email) values ($1) on conflict (email) do nothing", &[&email]).await.map(|_| ()).map_err(|e| e.to_string())
            }
        }
    }

    /// Forget a channel its tower has closed.
    pub async fn remove(&self, channel: &str) -> Result<(), String> {
        match self {
            Store::Folder(dir) => {
                let file = dir.join("channels.json");
                let mut all: HashMap<String, String> = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
                all.remove(channel);
                let tmp = dir.join("channels.json.tmp");
                std::fs::write(&tmp, serde_json::to_string(&all).unwrap()).and_then(|_| std::fs::rename(&tmp, &file)).map_err(|e| format!("cannot save {}: {e}", file.display()))
            }
            Store::Postgres(pg) => {
                let db = pg.connect(&pg.name).await?;
                db.execute("delete from relay_channels where channel = $1", &[&channel]).await.map(|_| ()).map_err(|e| e.to_string())
            }
        }
    }

    /// Remember a new channel. Returns only once it is durable.
    pub async fn add(&self, channel: &str, pull_hash: &str) -> Result<(), String> {
        match self {
            Store::Folder(dir) => {
                let file = dir.join("channels.json");
                let mut all: HashMap<String, String> = std::fs::read_to_string(&file).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
                all.insert(channel.into(), pull_hash.into());
                let tmp = dir.join("channels.json.tmp");
                std::fs::write(&tmp, serde_json::to_string(&all).unwrap()).and_then(|_| std::fs::rename(&tmp, &file)).map_err(|e| format!("cannot save {}: {e}", file.display()))
            }
            Store::Postgres(pg) => {
                // one short connection per new channel: channels are opened rarely, and nothing is held open to go stale
                let db = pg.connect(&pg.name).await?;
                db.execute("insert into relay_channels (channel, pull_hash) values ($1, $2) on conflict (channel) do nothing", &[&channel, &pull_hash]).await.map(|_| ()).map_err(|e| e.to_string())
            }
        }
    }
}

/// Sealed requests kept for a tower that is away, to hand over in the order they came once it is back. The relay
/// cannot read them. Each has an id of the radio's choosing, so one sent twice is kept once.
impl Store {
    fn held_dir(dir: &std::path::Path, channel: &str) -> PathBuf {
        dir.join("held").join(channel)
    }

    /// Keep one. False when the channel already has as many as it may; true when it is kept, now or before.
    pub async fn hold(&self, channel: &str, id: &str, sealed: &[u8], most: usize) -> Result<bool, String> {
        match self {
            Store::Folder(dir) => {
                let held = Self::held_dir(dir, channel);
                std::fs::create_dir_all(&held).map_err(|e| e.to_string())?;
                let names: Vec<String> = std::fs::read_dir(&held).map_err(|e| e.to_string())?.flatten().map(|e| e.file_name().to_string_lossy().to_string()).collect();
                if names.iter().any(|n| n.split_once('-').is_some_and(|(_, rest)| rest == id)) {
                    return Ok(true);
                }
                if names.len() >= most {
                    return Ok(false);
                }
                // in the order they came: a number that only goes up, then the id
                let next = names.iter().filter_map(|n| n.split_once('-')?.0.parse::<u64>().ok()).max().map_or(1, |n| n + 1);
                let (tmp, file) = (held.join(format!(".{id}.tmp")), held.join(format!("{next:012}-{id}")));
                std::fs::write(&tmp, sealed).and_then(|_| std::fs::rename(&tmp, &file)).map(|_| true).map_err(|e| e.to_string())
            }
            Store::Postgres(pg) => {
                let db = pg.connect(&pg.name).await?;
                let have: i64 = db.query_one("select count(*) from relay_held where channel = $1", &[&channel]).await.map_err(|e| e.to_string())?.get(0);
                let already = db.query_opt("select 1 from relay_held where channel = $1 and id = $2", &[&channel, &id]).await.map_err(|e| e.to_string())?.is_some();
                if already {
                    return Ok(true);
                }
                if have as usize >= most {
                    return Ok(false);
                }
                db.execute("insert into relay_held (channel, id, sealed) values ($1, $2, $3) on conflict (channel, id) do nothing", &[&channel, &id, &sealed]).await.map(|_| true).map_err(|e| e.to_string())
            }
        }
    }

    /// The oldest one kept for a channel, if any.
    pub async fn held_next(&self, channel: &str) -> Result<Option<(String, Vec<u8>)>, String> {
        match self {
            Store::Folder(dir) => {
                let held = Self::held_dir(dir, channel);
                let mut names: Vec<String> = std::fs::read_dir(&held).map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().to_string()).filter(|n| !n.starts_with('.')).collect()).unwrap_or_default();
                names.sort();
                Ok(names.first().and_then(|n| Some((n.split_once('-')?.1.to_string(), std::fs::read(held.join(n)).ok()?))))
            }
            Store::Postgres(pg) => {
                let db = pg.connect(&pg.name).await?;
                let row = db.query_opt("select id, sealed from relay_held where channel = $1 order by seq limit 1", &[&channel]).await.map_err(|e| e.to_string())?;
                Ok(row.map(|r| (r.get(0), r.get(1))))
            }
        }
    }

    /// The tower has taken one: it is kept no longer.
    pub async fn held_done(&self, channel: &str, id: &str) -> Result<(), String> {
        match self {
            Store::Folder(dir) => {
                let held = Self::held_dir(dir, channel);
                for e in std::fs::read_dir(&held).map_err(|e| e.to_string())?.flatten() {
                    if e.file_name().to_string_lossy().split_once('-').is_some_and(|(_, rest)| rest == id) {
                        let _ = std::fs::remove_file(e.path());
                    }
                }
                Ok(())
            }
            Store::Postgres(pg) => {
                let db = pg.connect(&pg.name).await?;
                db.execute("delete from relay_held where channel = $1 and id = $2", &[&channel, &id]).await.map(|_| ()).map_err(|e| e.to_string())
            }
        }
    }

    /// Nothing is kept for ever: what a tower never came back for is dropped after this many days.
    pub async fn held_purge(&self, days: u64) -> Result<(), String> {
        match self {
            Store::Folder(dir) => {
                let Ok(channels) = std::fs::read_dir(dir.join("held")) else { return Ok(()) };
                for file in channels.flatten().filter_map(|c| std::fs::read_dir(c.path()).ok()).flat_map(|d| d.flatten()) {
                    if file.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age.as_secs() > days * 86_400) {
                        let _ = std::fs::remove_file(file.path());
                    }
                }
                Ok(())
            }
            Store::Postgres(pg) => {
                let db = pg.connect(&pg.name).await?;
                db.execute("delete from relay_held where held_at < now() - make_interval(days => $1)", &[&(days as i32)]).await.map(|_| ()).map_err(|e| e.to_string())
            }
        }
    }
}

impl Pg {
    async fn connect(&self, database: &str) -> Result<Client, String> {
        let mut cfg = Config::new();
        cfg.host(&self.host).port(self.port).user(&self.user).password(&self.pass).dbname(database).connect_timeout(std::time::Duration::from_secs(15));
        let why = |e: tokio_postgres::Error| format!("cannot connect to database {database} on {}: {e}", self.host);
        match &self.tls {
            Some(tls) => {
                let (client, conn) = cfg.connect(tokio_postgres_rustls::MakeRustlsConnect::new((**tls).clone())).await.map_err(why)?;
                tokio::spawn(conn);
                Ok(client)
            }
            None => {
                let (client, conn) = cfg.connect(NoTls).await.map_err(why)?;
                tokio::spawn(conn);
                Ok(client)
            }
        }
    }
}
