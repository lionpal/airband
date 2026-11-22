//! The relay's records kept in Postgres: the same jobs the folder and DynamoDB stores do, in its tables.
use super::*;
use std::sync::Arc;
use tokio_postgres::{Client, Config, NoTls};

/// A kept message still within its time ([`in_time`]), as a condition on `relay_held`, with the time given as
/// parameter `$at` in seconds. One place, so every query passes over the same ones.
fn in_time_sql(at: usize) -> String {
    format!("held_at > now() - make_interval(secs => ${at})")
}
const HELD_FOR: f64 = HELD_FOR_SECS as f64;

pub(crate) struct Pg {
    pub(super) host: String,
    port: u16,
    user: String,
    pass: String,
    pub(super) name: String,
    tls: Option<Arc<rustls::ClientConfig>>,
}

impl Pg {
    /// From the environment, for the server at `host` (DB_HOST). Fails at once, with the reason, when it is half
    /// set.
    pub(super) fn from_env(host: String) -> Result<Pg, String> {
        let tls = if var("DB_SSL").as_deref() == Some("off") {
            None
        } else {
            let ca = var("DB_CA").ok_or("DB_HOST is set, so set DB_CA to a PEM file of authorities to trust, or DB_SSL=off for a local database")?;
            let pem = airband_io::files::read_bytes(std::path::Path::new(&ca)).map_err(|e| format!("cannot read DB_CA {ca}: {e}"))?.ok_or_else(|| format!("DB_CA {ca} is not there"))?;
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
        Ok(Pg {
            host,
            port: match var("DB_PORT") {
                None => 5432,
                Some(p) => p.parse().map_err(|_| format!("DB_PORT \"{p}\" is not a port number"))?,
            },
            user: var("DB_USER").ok_or("DB_HOST is set but DB_USER is not")?,
            pass: var("DB_PASS").ok_or("DB_HOST is set but DB_PASS is not")?,
            name: var("DB_NAME").unwrap_or_else(|| "airband".into()),
            tls,
        })
    }
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

#[async_trait::async_trait]
impl ChannelStore for Pg {
    fn describe(&self) -> String {
        format!("the database {} on {}", self.name, self.host)
    }
    async fn load(&self) -> Result<HashMap<String, Saved>, String> {
        // a database cannot be created from inside itself: ask the server's own `postgres` database
        let admin = self.connect("postgres").await?;
        let exists = admin.query_opt("select 1 from pg_database where datname = $1", &[&self.name]).await.map_err(|e| e.to_string())?.is_some();
        if !exists {
            if !self.name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') {
                return Err(format!("DB_NAME \"{}\" must be lowercase letters, digits and underscores", self.name));
            }
            admin.batch_execute(&format!("create database {}", self.name)).await.map_err(|e| format!("cannot create database {}: {e}", self.name))?;
        }
        let db = self.connect(&self.name).await?;
        db.batch_execute(
            "create table if not exists relay_channels (channel text primary key, pull_hash text not null, radio_hash text not null, created_at timestamptz not null default now());
             create table if not exists waitlist (email text primary key, joined_at timestamptz not null default now());
             create table if not exists relay_closed (channel text primary key, closed_at timestamptz not null default now());
             create table if not exists relay_held (seq bigserial primary key, channel text not null, id text not null, sealed bytea not null, held_at timestamptz not null default now(), unique (channel, id))",
        )
        .await
        .map_err(|e| e.to_string())?;
        let rows = db.query("select channel, pull_hash, radio_hash from relay_channels where channel not in (select channel from relay_closed)", &[]).await.map_err(|e| e.to_string())?;
        Ok(rows.iter().map(|r| (r.get(0), Saved { pull_hash: r.get(1), radio_hash: r.get(2) })).collect())
    }
    async fn join_waitlist(&self, email: &str) -> Result<(), String> {
        let db = self.connect(&self.name).await?;
        db.execute("insert into waitlist (email) values ($1) on conflict (email) do nothing", &[&email]).await.map(|_| ()).map_err(|e| e.to_string())
    }
    async fn remove(&self, channel: &str) -> Result<(), String> {
        // the channel and what was kept for it, together or not at all
        // closed on record in the same transaction, under the lock keeping takes, so nothing is kept after it
        let mut db = self.connect(&self.name).await?;
        let tx = db.transaction().await.map_err(|e| e.to_string())?;
        tx.execute("select pg_advisory_xact_lock(hashtext($1))", &[&channel]).await.map_err(|e| e.to_string())?;
        tx.execute("insert into relay_closed (channel) values ($1) on conflict (channel) do nothing", &[&channel]).await.map_err(|e| e.to_string())?;
        tx.execute("delete from relay_held where channel = $1", &[&channel]).await.map_err(|e| e.to_string())?;
        tx.execute("delete from relay_channels where channel = $1", &[&channel]).await.map_err(|e| e.to_string())?;
        tx.commit().await.map_err(|e| e.to_string())
    }
    async fn add(&self, channel: &str, pull_hash: &str, radio_hash: &str) -> Result<Added, String> {
        // one short connection per new channel: channels are opened rarely, and nothing is held open to go stale
        let mut db = self.connect(&self.name).await?;
        let tx = db.transaction().await.map_err(|e| e.to_string())?;
        tx.execute("select pg_advisory_xact_lock(hashtext($1))", &[&channel]).await.map_err(|e| e.to_string())?;
        if tx.query_opt("select 1 from relay_closed where channel = $1", &[&channel]).await.map_err(|e| e.to_string())?.is_some() {
            return Ok(Added::Closed);
        }
        tx.execute("insert into relay_channels (channel, pull_hash, radio_hash) values ($1, $2, $3) on conflict (channel) do nothing", &[&channel, &pull_hash, &radio_hash]).await.map_err(|e| e.to_string())?;
        let row = tx.query_one("select pull_hash, radio_hash from relay_channels where channel = $1", &[&channel]).await.map_err(|e| e.to_string())?;
        let on_record = Saved { pull_hash: row.get(0), radio_hash: row.get(1) };
        tx.commit().await.map(|_| Added::Open(on_record)).map_err(|e| e.to_string())
    }
    async fn hold(&self, channel: &str, id: &str, sealed: &[u8], most: usize) -> Result<Held, String> {
        // counted and kept in one transaction, under a lock on the channel held until it ends, so two at
        // once cannot both see room for one more and both be kept
        let mut db = self.connect(&self.name).await?;
        let tx = db.transaction().await.map_err(|e| e.to_string())?;
        tx.execute("select pg_advisory_xact_lock(hashtext($1))", &[&channel]).await.map_err(|e| e.to_string())?;
        if tx.query_opt("select 1 from relay_closed where channel = $1", &[&channel]).await.map_err(|e| e.to_string())?.is_some() {
            return Ok(Held::Closed);
        }
        // the same id past its time goes, and this one is kept as new, as every store does
        tx.execute(&format!("delete from relay_held where channel = $1 and id = $2 and not ({})", in_time_sql(3)), &[&channel, &id, &HELD_FOR]).await.map_err(|e| e.to_string())?;
        let already = tx.query_opt("select 1 from relay_held where channel = $1 and id = $2", &[&channel, &id]).await.map_err(|e| e.to_string())?.is_some();
        if already {
            return Ok(Held::Kept);
        }
        let have: i64 = tx.query_one(&format!("select count(*) from relay_held where channel = $1 and {}", in_time_sql(2)), &[&channel, &HELD_FOR]).await.map_err(|e| e.to_string())?.get(0);
        if have as usize >= most {
            return Ok(Held::Full);
        }
        tx.execute("insert into relay_held (channel, id, sealed) values ($1, $2, $3) on conflict (channel, id) do nothing", &[&channel, &id, &sealed]).await.map_err(|e| e.to_string())?;
        tx.commit().await.map(|_| Held::Kept).map_err(|e| e.to_string())
    }
    async fn held_next(&self, channel: &str) -> Result<Option<(String, Vec<u8>)>, String> {
        let db = self.connect(&self.name).await?;
        let row = db.query_opt(&format!("select id, sealed from relay_held where channel = $1 and {} order by seq limit 1", in_time_sql(2)), &[&channel, &HELD_FOR]).await.map_err(|e| e.to_string())?;
        Ok(row.map(|r| (r.get(0), r.get(1))))
    }
    async fn held_done(&self, channel: &str, id: &str) -> Result<(), String> {
        let db = self.connect(&self.name).await?;
        db.execute("delete from relay_held where channel = $1 and id = $2", &[&channel, &id]).await.map(|_| ()).map_err(|e| e.to_string())
    }
    async fn held_purge(&self) -> Result<(), String> {
        let db = self.connect(&self.name).await?;
        db.execute(&format!("delete from relay_held where not ({})", in_time_sql(1)), &[&HELD_FOR]).await.map(|_| ()).map_err(|e| e.to_string())
    }
}
