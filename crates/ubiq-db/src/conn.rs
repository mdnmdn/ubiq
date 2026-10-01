//! `conn` — the connection model and connection-string parsing.
//!
//! [`ConnectionConfig`] is what the connection dialog edits, what is persisted (serde, JSON) and
//! what [`crate::driver::connect`] consumes. [`parse_connection_string`] turns whatever the user
//! pasted into one, auto-detecting the format:
//!
//! | Format | Examples |
//! |---|---|
//! | URL | `postgres://u:p@h:5432/db?sslmode=require`, `mysql://…`, `mariadb://…`, `sqlserver://…`, `mssql://…`, `sqlite:///abs/x.db`, `sqlite:rel.db`, `sqlite::memory:`, `file:x.db?mode=ro` |
//! | Bare SQLite path | `/data/app.db`, `C:\data\app.sqlite3`, `:memory:` (extensions `.db .db3 .sqlite .sqlite3`) |
//! | libpq key=value | `host=h port=5432 dbname=db user=u password='a b'` (always Postgres) |
//! | ADO.NET / ODBC | `Server=h,1433;Database=db;User Id=u;Password=p`, `Driver={PostgreSQL Unicode};Server=h;…`, `Data Source=app.db` |
//! | JDBC | `jdbc:postgresql://…`, `jdbc:mysql://…`, `jdbc:mariadb://…`, `jdbc:sqlserver://h\inst:1433;databaseName=db;user=u`, `jdbc:sqlite:path` |
//!
//! Keys are case-insensitive and compared with spaces, `_` and `-` removed (`User Id` = `userid` =
//! `USER_ID`). Values may be quoted `'…'`, `"…"` or `{…}` (doubled closing char escapes it).
//! URL parts and query values are percent-decoded. Unknown keys are kept in
//! [`ConnectionConfig::params`] under their original spelling.
//!
//! The semicolon form names no engine by itself. Its kind comes, in order, from a `Provider=` /
//! `Driver=` value, then the caller's hint ([`parse_connection_string_with`]), then the keys:
//! a SQLite-looking `Data Source` → SQLite; `Initial Catalog`, `Integrated Security`,
//! `Trusted_Connection`, `TrustServerCertificate`, `Encrypt`, `host,port` or `host\instance` →
//! SQL Server; `Host` / `Username` → Postgres (Npgsql); `Uid` → MySQL (Connector/NET); a well-known
//! port; and finally SQL Server, ADO.NET's home.
//!
//! # Read-only
//!
//! [`ConnectionConfig::read_only`] is set by any of: `readonly` / `read_only` / `read-only` /
//! `Read Only` / JDBC `readOnly` = `true|1|yes|on` (every format, every engine), SQLite
//! `mode=ro` / `Mode=ReadOnly` (consumed, not kept in `params`), and SQL Server
//! `ApplicationIntent=ReadOnly` — which is routing intent on SQL Server (a readable secondary),
//! taken here as read-only as well. [`ConnectionConfig::to_url`] writes it back as
//! `readonly=true`. What a read-only connection does is the driver's business
//! ([`crate::driver`], "Read-only").

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The four engines.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DbKind {
    Postgres,
    /// MySQL and MariaDB — one driver, one dialect.
    MySql,
    Sqlite,
    MsSql,
}

impl DbKind {
    pub const ALL: [DbKind; 4] = [
        DbKind::Postgres,
        DbKind::MySql,
        DbKind::Sqlite,
        DbKind::MsSql,
    ];

    /// The engine's default TCP port; `None` for SQLite (a file).
    pub fn default_port(self) -> Option<u16> {
        match self {
            DbKind::Postgres => Some(5432),
            DbKind::MySql => Some(3306),
            DbKind::Sqlite => None,
            DbKind::MsSql => Some(1433),
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            DbKind::Postgres => "PostgreSQL",
            DbKind::MySql => "MySQL / MariaDB",
            DbKind::Sqlite => "SQLite",
            DbKind::MsSql => "SQL Server",
        }
    }

    /// The scheme [`ConnectionConfig::to_url`] writes.
    pub fn url_scheme(self) -> &'static str {
        match self {
            DbKind::Postgres => "postgres",
            DbKind::MySql => "mysql",
            DbKind::Sqlite => "sqlite",
            DbKind::MsSql => "sqlserver",
        }
    }

    /// SQLite is a file path; the others are host/port/database/user.
    pub fn is_file_based(self) -> bool {
        self == DbKind::Sqlite
    }
}

impl fmt::Display for DbKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.display_name())
    }
}

/// Transport encryption, reduced to what all three network engines can express.
///
/// Mapping in: libpq `disable` / `allow`,`prefer` / `require` / `verify-ca`,`verify-full`;
/// MySQL `DISABLED` / `PREFERRED` / `REQUIRED` / `VERIFY_CA`,`VERIFY_IDENTITY`;
/// SQL Server `Encrypt=false` → `Disable`, `Encrypt=true;TrustServerCertificate=true` →
/// `Require`, `Encrypt=true|strict` → `VerifyFull`. Ignored for SQLite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SslMode {
    /// Plain TCP.
    Disable,
    /// TLS if the server offers it, otherwise plain. The engine default.
    #[default]
    Prefer,
    /// TLS required, certificate not verified.
    Require,
    /// TLS required, certificate chain and host name verified.
    VerifyFull,
}

/// One saved connection. Every field but `name` and `kind` is optional; what an engine needs is
/// checked by [`ConnectionConfig::validate`].
///
/// The password is stored as given — a host that files the config strips it first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionConfig {
    /// User-facing label; [`parse_connection_string`] fills it from [`Self::default_name`].
    #[serde(default)]
    pub name: String,
    pub kind: DbKind,
    /// Host name, IP, or (Postgres) a socket directory. `None` means `localhost`
    /// ([`Self::effective_host`]). Network engines only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// `None` means the engine default ([`Self::effective_port`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// SQL Server named instance (`host\instance`). SQL Server only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,
    /// Initial database. Postgres: the one to connect to (`postgres` when absent is the driver's
    /// call); MySQL / SQL Server: the default database.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// The database file. SQLite only, and required there (`:memory:` allowed).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    #[serde(default)]
    pub ssl: SslMode,
    /// SQL Server `Integrated Security` / Windows auth: no user/password.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub integrated_auth: bool,
    /// Open read-only: every statement on the connection runs under the driver's read-only
    /// guard ([`crate::driver::ExecOptions::read_only`] forced on). See the module docs for the
    /// keys that set it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub read_only: bool,
    /// Every key the parser did not map to a field, original spelling, decoded value
    /// (`application_name`, `connect_timeout`, …). Drivers read what they know.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, String>,
}

/// Why a connection string did not parse, or a config did not validate.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    #[error("the connection string is empty")]
    Empty,
    #[error("unrecognised connection string format")]
    UnknownFormat,
    #[error("unsupported scheme `{0}`")]
    UnknownScheme(String),
    #[error("invalid port `{0}`")]
    InvalidPort(String),
    #[error("{0}")]
    Malformed(String),
    /// A field the engine cannot do without (`"path"` for SQLite).
    #[error("missing {0}")]
    Missing(&'static str),
}

impl ConnectionConfig {
    /// An empty config of `kind`: every optional field unset, `ssl` at its default.
    pub fn new(kind: DbKind) -> Self {
        Self {
            name: String::new(),
            kind,
            host: None,
            port: None,
            instance: None,
            database: None,
            user: None,
            password: None,
            path: None,
            ssl: SslMode::default(),
            integrated_auth: false,
            read_only: false,
            params: BTreeMap::new(),
        }
    }

    /// `host`, or `localhost` when unset.
    pub fn effective_host(&self) -> &str {
        self.host
            .as_deref()
            .filter(|h| !h.is_empty())
            .unwrap_or("localhost")
    }

    /// `port`, or the engine default. `None` for SQLite, and for a SQL Server named instance
    /// without an explicit port (the SQL Browser service resolves it).
    pub fn effective_port(&self) -> Option<u16> {
        if self.port.is_some() {
            return self.port;
        }
        if self.kind == DbKind::MsSql && self.instance.is_some() {
            return None;
        }
        self.kind.default_port()
    }

    /// `db @ host` for a server, the file name for SQLite.
    pub fn default_name(&self) -> String {
        if self.kind == DbKind::Sqlite {
            return self
                .path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .or_else(|| self.path.as_ref().map(|p| p.to_string_lossy().into_owned()))
                .unwrap_or_else(|| "SQLite".into());
        }
        let mut host = self.effective_host().to_string();
        if let Some(inst) = &self.instance {
            host = format!("{host}\\{inst}");
        }
        match self.database.as_deref().filter(|d| !d.is_empty()) {
            Some(db) => format!("{db} @ {host}"),
            None => host,
        }
    }

    /// What `kind` cannot do without. SQLite: a non-empty `path`. Network engines: no zero port,
    /// `instance` only on SQL Server. A missing host is fine (`localhost`).
    pub fn validate(&self) -> Result<(), ParseError> {
        if self.kind == DbKind::Sqlite {
            let ok = self
                .path
                .as_ref()
                .is_some_and(|p| !p.as_os_str().is_empty());
            return if ok {
                Ok(())
            } else {
                Err(ParseError::Missing("path"))
            };
        }
        if self.port == Some(0) {
            return Err(ParseError::InvalidPort("0".into()));
        }
        if self.instance.is_some() && self.kind != DbKind::MsSql {
            return Err(ParseError::Malformed(
                "a named instance is SQL Server only".into(),
            ));
        }
        Ok(())
    }

    /// The config as a URL in this crate's own dialect, for display and copy — it parses back
    /// through [`parse_connection_string`] to the same config (except `name`). With
    /// `mask_password` the password is written `***`.
    pub fn to_url(&self, mask_password: bool) -> String {
        let mut query: Vec<(String, String)> = Vec::new();
        let mut out = String::new();
        if self.kind == DbKind::Sqlite {
            let p = self
                .path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let enc = pct_encode(&p, b"/:\\");
            out.push_str(if p.starts_with('/') {
                "sqlite://"
            } else {
                "sqlite:"
            });
            out.push_str(&enc);
        } else {
            out.push_str(self.kind.url_scheme());
            out.push_str("://");
            if let Some(u) = &self.user {
                out.push_str(&pct_encode(u, b""));
                if let Some(p) = &self.password {
                    out.push(':');
                    out.push_str(&if mask_password {
                        "***".into()
                    } else {
                        pct_encode(p, b"")
                    });
                }
                out.push('@');
            }
            let host = self.host.as_deref().unwrap_or("");
            if host.contains(':') {
                out.push('[');
                out.push_str(host);
                out.push(']');
            } else {
                out.push_str(&pct_encode(host, b",."));
            }
            if let Some(inst) = &self.instance {
                out.push_str("%5C");
                out.push_str(&pct_encode(inst, b""));
            }
            if let Some(port) = self.port {
                out.push_str(&format!(":{port}"));
            }
            if let Some(db) = &self.database {
                out.push('/');
                out.push_str(&pct_encode(db, b""));
            }
            let ssl = match (self.kind, self.ssl) {
                (_, SslMode::Prefer) => None,
                (DbKind::Postgres, m) => Some(("sslmode", m.libpq_name())),
                (DbKind::MySql, m) => Some(("ssl-mode", m.mysql_name())),
                _ => None,
            };
            if let Some((k, v)) = ssl {
                query.push((k.into(), v.into()));
            }
            if self.kind == DbKind::MsSql {
                match self.ssl {
                    SslMode::Prefer => {}
                    SslMode::Disable => query.push(("encrypt".into(), "false".into())),
                    SslMode::Require => {
                        query.push(("encrypt".into(), "true".into()));
                        query.push(("trustServerCertificate".into(), "true".into()));
                    }
                    SslMode::VerifyFull => query.push(("encrypt".into(), "true".into())),
                }
                if self.integrated_auth {
                    query.push(("integratedSecurity".into(), "true".into()));
                }
            }
        }
        if self.read_only {
            query.push(("readonly".into(), "true".into()));
        }
        query.extend(self.params.iter().map(|(k, v)| (k.clone(), v.clone())));
        for (i, (k, v)) in query.iter().enumerate() {
            out.push(if i == 0 { '?' } else { '&' });
            out.push_str(&pct_encode(k, b""));
            out.push('=');
            out.push_str(&pct_encode(v, b""));
        }
        out
    }
}

impl SslMode {
    fn libpq_name(self) -> &'static str {
        match self {
            SslMode::Disable => "disable",
            SslMode::Prefer => "prefer",
            SslMode::Require => "require",
            SslMode::VerifyFull => "verify-full",
        }
    }

    fn mysql_name(self) -> &'static str {
        match self {
            SslMode::Disable => "DISABLED",
            SslMode::Prefer => "PREFERRED",
            SslMode::Require => "REQUIRED",
            SslMode::VerifyFull => "VERIFY_IDENTITY",
        }
    }

    fn parse(v: &str) -> Option<SslMode> {
        let v = v.trim().to_ascii_lowercase().replace('_', "-");
        Some(match v.as_str() {
            "disable" | "disabled" | "false" | "no" | "off" | "none" | "0" => SslMode::Disable,
            "allow" | "prefer" | "preferred" | "optional" => SslMode::Prefer,
            "require" | "required" | "true" | "yes" | "on" | "1" | "mandatory" => SslMode::Require,
            "verify-ca" | "verify-full" | "verify-identity" | "strict" => SslMode::VerifyFull,
            _ => return None,
        })
    }
}

/// Parse a connection string in any supported format (see the module docs), then
/// [`validate`](ConnectionConfig::validate) it. `name` is set to [`ConnectionConfig::default_name`].
pub fn parse_connection_string(input: &str) -> Result<ConnectionConfig, ParseError> {
    parse_connection_string_with(input, None)
}

/// As [`parse_connection_string`], with the kind the user already picked. The hint only decides
/// what the string itself leaves open: a semicolon form with no `Provider`/`Driver`, or a bare
/// path with no SQLite extension (`hint = Sqlite`). A URL's scheme always wins.
pub fn parse_connection_string_with(
    input: &str,
    hint: Option<DbKind>,
) -> Result<ConnectionConfig, ParseError> {
    let s = input.trim();
    if s.is_empty() {
        return Err(ParseError::Empty);
    }
    let mut cfg = if let Some(cfg) = parse_schemed(s)? {
        cfg
    } else if is_sqlite_path(s) || (hint == Some(DbKind::Sqlite) && !s.contains('=')) {
        let mut c = ConnectionConfig::new(DbKind::Sqlite);
        c.path = Some(PathBuf::from(s));
        c
    } else if s.contains('=') {
        // A `;` anywhere means the semicolon form (Npgsql's `Host=h;Port=…` would otherwise
        // read as one libpq `host` value).
        match if s.contains(';') {
            None
        } else {
            parse_libpq(s)?
        } {
            Some(c) => c,
            None => parse_semicolon(s, hint)?,
        }
    } else {
        return Err(ParseError::UnknownFormat);
    };
    if cfg.name.is_empty() {
        cfg.name = cfg.default_name();
    }
    cfg.validate()?;
    Ok(cfg)
}

/// `scheme:` forms — URLs and JDBC. `None` when `s` does not start with a known scheme.
fn parse_schemed(s: &str) -> Result<Option<ConnectionConfig>, ParseError> {
    let Some((scheme, rest)) = s.split_once(':') else {
        return Ok(None);
    };
    let valid = !scheme.is_empty()
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c));
    if !valid {
        return Ok(None);
    }
    let lower = scheme.to_ascii_lowercase();
    let kind = match lower.as_str() {
        "jdbc" => {
            return match parse_schemed(rest)? {
                Some(c) => Ok(Some(c)),
                None => Err(ParseError::UnknownScheme(format!("jdbc:{rest}"))),
            };
        }
        "sqlite" | "sqlite3" | "file" => return parse_sqlite_url(rest).map(Some),
        "postgres" | "postgresql" | "pgsql" => DbKind::Postgres,
        "mysql" | "mariadb" | "mysqlx" => DbKind::MySql,
        "sqlserver" | "mssql" => DbKind::MsSql,
        _ if rest.starts_with("//") => return Err(ParseError::UnknownScheme(scheme.into())),
        _ => return Ok(None),
    };
    let Some(rest) = rest.strip_prefix("//") else {
        return Err(ParseError::Malformed(format!(
            "expected `{scheme}://` followed by a host"
        )));
    };
    let mut b = Builder::new(kind);
    // JDBC SQL Server: `host[\inst][:port];key=value;…`, no path, no query.
    if kind == DbKind::MsSql && rest.contains(';') {
        let (server, props) = rest.split_once(';').unwrap_or((rest, ""));
        if !server.is_empty() {
            b.server = Some(pct_decode(server)?);
        }
        for (k, v) in split_pairs(props)? {
            b.apply(&k, v)?;
        }
        return b.finish().map(Some);
    }
    let rest = rest.split_once('#').map_or(rest, |(a, _)| a);
    let (main, query) = rest.split_once('?').unwrap_or((rest, ""));
    let (authority, path) = main.split_once('/').unwrap_or((main, ""));
    let hostport = match authority.rsplit_once('@') {
        Some((userinfo, hp)) => {
            let (u, p) = match userinfo.split_once(':') {
                Some((u, p)) => (u, Some(p)),
                None => (userinfo, None),
            };
            if !u.is_empty() {
                b.cfg.user = Some(pct_decode(u)?);
            }
            if let Some(p) = p {
                b.cfg.password = Some(pct_decode(p)?);
            }
            hp
        }
        None => authority,
    };
    if !hostport.is_empty() {
        let (host, port) = split_host_port(hostport, false);
        let host = pct_decode(host)?;
        if !host.is_empty() {
            b.server = Some(host);
        }
        if let Some(p) = port {
            b.cfg.port = Some(parse_port(p)?);
        }
    }
    let db = pct_decode(path.trim_end_matches('/'))?;
    if !db.is_empty() {
        b.cfg.database = Some(db);
    }
    apply_query(&mut b, query)?;
    b.finish().map(Some)
}

fn apply_query(b: &mut Builder, query: &str) -> Result<(), ParseError> {
    for pair in query.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        b.apply(&pct_decode(k)?, pct_decode(v)?)?;
    }
    Ok(())
}

/// After `sqlite:` / `file:`: `//rel.db`, `///abs.db`, `rel.db`, `:memory:`, each with `?query`.
fn parse_sqlite_url(rest: &str) -> Result<ConnectionConfig, ParseError> {
    let (p, query) = rest.split_once('?').unwrap_or((rest, ""));
    let mut p = p.strip_prefix("//").unwrap_or(p);
    if p.starts_with("localhost/") {
        p = &p["localhost".len()..];
    }
    let mut b = Builder::new(DbKind::Sqlite);
    let path = pct_decode(p)?;
    if !path.is_empty() {
        b.cfg.path = Some(PathBuf::from(path));
    }
    apply_query(&mut b, query)?;
    b.finish()
}

fn is_sqlite_path(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    !s.contains('=')
        && (l == ":memory:"
            || [".db", ".db3", ".sqlite", ".sqlite3"]
                .iter()
                .any(|e| l.ends_with(e)))
}

/// libpq `key=value key='quoted \' value'`. `None` when the text is not that shape, or names no
/// libpq key — the caller then tries the semicolon form.
fn parse_libpq(s: &str) -> Result<Option<ConnectionConfig>, ParseError> {
    const LIBPQ: &[&str] = &[
        "host", "hostaddr", "port", "dbname", "user", "password", "sslmode",
    ];
    let mut pairs = Vec::new();
    let mut it = s.chars().peekable();
    loop {
        while it.next_if(|c| c.is_whitespace()).is_some() {}
        if it.peek().is_none() {
            break;
        }
        let mut key = String::new();
        while let Some(c) = it.next_if(|c| c.is_ascii_alphanumeric() || *c == '_') {
            key.push(c);
        }
        while it.next_if(|c| c.is_whitespace()).is_some() {}
        if key.is_empty() || it.next() != Some('=') {
            return Ok(None);
        }
        while it.next_if(|c| c.is_whitespace()).is_some() {}
        let mut val = String::new();
        if it.next_if_eq(&'\'').is_some() {
            loop {
                match it.next() {
                    Some('\\') => val.extend(it.next()),
                    Some('\'') => break,
                    Some(c) => val.push(c),
                    None => {
                        return Err(ParseError::Malformed(format!(
                            "unterminated quote in `{key}`"
                        )));
                    }
                }
            }
        } else {
            while let Some(c) = it.next_if(|c| !c.is_whitespace()) {
                if c == '\\' {
                    val.extend(it.next());
                } else {
                    val.push(c);
                }
            }
        }
        pairs.push((key, val));
    }
    let known = pairs
        .iter()
        .any(|(k, _)| LIBPQ.contains(&k.to_ascii_lowercase().as_str()));
    if !known {
        return Ok(None);
    }
    let mut b = Builder::new(DbKind::Postgres);
    for (k, v) in pairs {
        b.apply(&k, v)?;
    }
    b.finish().map(Some)
}

/// ADO.NET / ODBC `key=value;key=value`.
fn parse_semicolon(s: &str, hint: Option<DbKind>) -> Result<ConnectionConfig, ParseError> {
    let pairs = split_pairs(s)?;
    if pairs.is_empty() {
        return Err(ParseError::UnknownFormat);
    }
    let kind = infer_kind(&pairs, hint);
    let mut b = Builder::new(kind);
    for (k, v) in pairs {
        b.apply(&k, v)?;
    }
    b.finish()
}

fn infer_kind(pairs: &[(String, String)], hint: Option<DbKind>) -> DbKind {
    let get = |names: &[&str]| {
        pairs
            .iter()
            .find(|(k, _)| names.contains(&norm_key(k).as_str()))
            .map(|(_, v)| v.as_str())
    };
    if let Some(p) = get(&["provider", "driver"]) {
        let p = p.to_ascii_lowercase();
        if p.contains("sqlite") {
            return DbKind::Sqlite;
        }
        if p.contains("postgres") || p.contains("npgsql") || p.contains("psql") {
            return DbKind::Postgres;
        }
        if p.contains("mysql") || p.contains("mariadb") {
            return DbKind::MySql;
        }
        if [
            "sql server",
            "sqlserver",
            "sqlncli",
            "msoledbsql",
            "sqloledb",
            "sqlclient",
        ]
        .iter()
        .any(|n| p.contains(n))
        {
            return DbKind::MsSql;
        }
    }
    if let Some(h) = hint {
        return h;
    }
    let server = get(&["datasource", "server", "address", "addr", "networkaddress"]);
    if server.is_some_and(is_sqlite_path) {
        return DbKind::Sqlite;
    }
    let mssql_keys = [
        "initialcatalog",
        "integratedsecurity",
        "trustedconnection",
        "trustservercertificate",
        "encrypt",
        "multipleactiveresultsets",
        "applicationintent",
    ];
    if get(&mssql_keys).is_some() || server.is_some_and(|s| s.contains(',') || s.contains('\\')) {
        return DbKind::MsSql;
    }
    if get(&["host", "username"]).is_some() {
        return DbKind::Postgres;
    }
    if get(&["uid"]).is_some() {
        return DbKind::MySql;
    }
    match get(&["port"]).map(str::trim) {
        Some("5432") => DbKind::Postgres,
        Some("3306") => DbKind::MySql,
        _ => DbKind::MsSql,
    }
}

/// `key=value;…` with `'…'`, `"…"` and `{…}` quoting. Empty segments are skipped.
fn split_pairs(s: &str) -> Result<Vec<(String, String)>, ParseError> {
    let mut out = Vec::new();
    let mut it = s.chars().peekable();
    loop {
        while it.next_if(|c| c.is_whitespace() || *c == ';').is_some() {}
        if it.peek().is_none() {
            break;
        }
        let mut key = String::new();
        while let Some(c) = it.next_if(|c| *c != '=' && *c != ';') {
            key.push(c);
        }
        let key = key.trim().to_string();
        if it.next() != Some('=') {
            return Err(ParseError::Malformed(format!("`{key}` has no value")));
        }
        while it.next_if(|c| *c == ' ' || *c == '\t').is_some() {}
        let close = match it.peek() {
            Some('{') => Some('}'),
            Some('\'') => Some('\''),
            Some('"') => Some('"'),
            _ => None,
        };
        let mut val = String::new();
        if let Some(close) = close {
            it.next();
            loop {
                match it.next() {
                    Some(c) if c == close => {
                        if it.next_if_eq(&close).is_some() {
                            val.push(close);
                        } else {
                            break;
                        }
                    }
                    Some(c) => val.push(c),
                    None => {
                        return Err(ParseError::Malformed(format!(
                            "unterminated quote in `{key}`"
                        )));
                    }
                }
            }
            while it.next_if(|c| *c != ';').is_some() {}
        } else {
            while let Some(c) = it.next_if(|c| *c != ';') {
                val.push(c);
            }
            val = val.trim().to_string();
        }
        out.push((key, val));
    }
    Ok(out)
}

/// Case-insensitive key with spaces, `_` and `-` removed.
fn norm_key(k: &str) -> String {
    k.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Accumulates keys from any format; `finish` resolves what depends on the kind.
struct Builder {
    cfg: ConnectionConfig,
    /// Raw server spec (`host`, `host,port`, `host\inst`, `tcp:host`); a path for SQLite.
    server: Option<String>,
    encrypt: Option<bool>,
    trust: Option<bool>,
}

impl Builder {
    fn new(kind: DbKind) -> Self {
        Self {
            cfg: ConnectionConfig::new(kind),
            server: None,
            encrypt: None,
            trust: None,
        }
    }

    fn apply(&mut self, key: &str, val: String) -> Result<(), ParseError> {
        let c = &mut self.cfg;
        match norm_key(key).as_str() {
            "server" | "datasource" | "address" | "addr" | "networkaddress" | "host"
            | "hostname" | "servername" => self.server = Some(val),
            "hostaddr" => {
                if self.server.is_none() {
                    self.server = Some(val);
                }
            }
            "port" | "portnumber" => c.port = Some(parse_port(&val)?),
            "database" | "initialcatalog" | "dbname" | "databasename" => c.database = Some(val),
            "user" | "userid" | "uid" | "username" => c.user = Some(val),
            "password" | "pwd" => c.password = Some(val),
            "sslmode" => {
                c.ssl = SslMode::parse(&val)
                    .ok_or_else(|| ParseError::Malformed(format!("unknown ssl mode `{val}`")))?
            }
            "ssl" | "usessl" => {
                c.ssl = if parse_bool(key, &val)? {
                    SslMode::Require
                } else {
                    SslMode::Disable
                }
            }
            "encrypt" => {
                self.encrypt = Some(match val.trim().to_ascii_lowercase().as_str() {
                    "strict" | "mandatory" => true,
                    "optional" => false,
                    _ => parse_bool(key, &val)?,
                })
            }
            "trustservercertificate" => self.trust = Some(parse_bool(key, &val)?),
            "integratedsecurity" | "trustedconnection" => {
                c.integrated_auth = parse_bool(key, &val)?
            }
            "instance" | "instancename" => c.instance = Some(val),
            "readonly" => c.read_only = parse_bool(key, &val)?,
            // SQL Server routing intent; `ReadWrite` is the default and says nothing.
            "applicationintent" => {
                if norm_key(&val) == "readonly" {
                    c.read_only = true;
                }
            }
            // SQLite URI `mode=ro`, System.Data.SQLite / Microsoft.Data.Sqlite `Mode=ReadOnly`.
            "mode" if matches!(norm_key(&val).as_str(), "ro" | "readonly") => c.read_only = true,
            "provider" | "driver" => {}
            _ => {
                c.params.insert(key.to_string(), val);
            }
        }
        Ok(())
    }

    fn finish(mut self) -> Result<ConnectionConfig, ParseError> {
        let kind = self.cfg.kind;
        // ODBC SQLite drivers name the file `Database=`.
        if kind == DbKind::Sqlite && self.cfg.path.is_none() && self.server.is_none() {
            self.cfg.path = self.cfg.database.take().map(PathBuf::from);
        }
        if let Some(server) = self.server.take() {
            if kind == DbKind::Sqlite {
                if self.cfg.path.is_none() && !server.is_empty() {
                    self.cfg.path = Some(PathBuf::from(server));
                }
            } else {
                let s = server.trim();
                let s = s
                    .strip_prefix("tcp:")
                    .or_else(|| s.strip_prefix("TCP:"))
                    .unwrap_or(s);
                let (hostinst, port) = split_host_port(s, kind == DbKind::MsSql);
                let (host, inst) = match hostinst.split_once('\\') {
                    Some((h, i)) => (h, Some(i)),
                    None => (hostinst, None),
                };
                let host = match host.to_ascii_lowercase().as_str() {
                    "." | "(local)" | "" => "localhost".to_string(),
                    _ => host.to_string(),
                };
                self.cfg.host = Some(host);
                if let Some(i) = inst.filter(|i| !i.is_empty()) {
                    self.cfg.instance = Some(i.to_string());
                }
                if let (Some(p), None) = (port, self.cfg.port) {
                    self.cfg.port = Some(parse_port(p)?);
                }
            }
        }
        self.cfg.ssl = match (self.encrypt, self.trust.unwrap_or(false)) {
            (Some(false), _) => SslMode::Disable,
            (Some(true), true) | (None, true) => SslMode::Require,
            (Some(true), false) => SslMode::VerifyFull,
            (None, false) => self.cfg.ssl,
        };
        Ok(self.cfg)
    }
}

/// `host:port`, `[v6]:port`, and with `comma_port` SQL Server's `host,port`. A host with more
/// than one `:` and no brackets (bare IPv6, Postgres multi-host) is left whole.
fn split_host_port(s: &str, comma_port: bool) -> (&str, Option<&str>) {
    if let Some((h, after)) = s.strip_prefix('[').and_then(|rest| rest.split_once(']')) {
        return (h, after.strip_prefix(':').filter(|p| !p.is_empty()));
    }
    if let Some((h, p)) = s.rsplit_once(',').filter(|_| comma_port) {
        return (h.trim(), Some(p.trim()));
    }
    if s.matches(':').count() == 1 {
        let (h, p) = s.split_once(':').unwrap_or((s, ""));
        return (h, Some(p).filter(|p| !p.is_empty()));
    }
    (s, None)
}

fn parse_port(p: &str) -> Result<u16, ParseError> {
    match p.trim().parse::<u16>() {
        Ok(n) if n > 0 => Ok(n),
        _ => Err(ParseError::InvalidPort(p.to_string())),
    }
}

fn parse_bool(key: &str, v: &str) -> Result<bool, ParseError> {
    match v.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "1" | "on" | "sspi" => Ok(true),
        "false" | "no" | "0" | "off" => Ok(false),
        _ => Err(ParseError::Malformed(format!(
            "`{key}`: expected true/false, got `{v}`"
        ))),
    }
}

fn pct_decode(s: &str) -> Result<String, ParseError> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = b
                .get(i + 1..i + 3)
                .and_then(|h| std::str::from_utf8(h).ok());
            let byte = hex.and_then(|h| u8::from_str_radix(h, 16).ok());
            let Some(byte) = byte else {
                return Err(ParseError::Malformed(format!(
                    "bad percent-escape in `{s}`"
                )));
            };
            out.push(byte);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| ParseError::Malformed(format!("`{s}` is not UTF-8")))
}

/// Percent-encode everything but RFC 3986 unreserved characters and `keep`.
fn pct_encode(s: &str, keep: &[u8]) -> String {
    let mut out = String::with_capacity(s.len());
    for &b in s.as_bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) || keep.contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> ConnectionConfig {
        parse_connection_string(s).unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    fn some(s: &str) -> Option<String> {
        Some(s.to_string())
    }

    #[test]
    fn postgres_url_full() {
        let c = p(
            "postgres://alice:s%40cret@db.example.com:6543/shop?sslmode=require&application_name=dbx",
        );
        assert_eq!(c.kind, DbKind::Postgres);
        assert_eq!(c.host, some("db.example.com"));
        assert_eq!(c.port, Some(6543));
        assert_eq!(c.user, some("alice"));
        assert_eq!(c.password, some("s@cret"));
        assert_eq!(c.database, some("shop"));
        assert_eq!(c.ssl, SslMode::Require);
        assert_eq!(c.params.get("application_name"), Some(&"dbx".to_string()));
        assert_eq!(c.name, "shop @ db.example.com");
    }

    #[test]
    fn postgresql_url_minimal() {
        let c = p("postgresql://localhost");
        assert_eq!(c.kind, DbKind::Postgres);
        assert_eq!(c.host, some("localhost"));
        assert_eq!(c.port, None);
        assert_eq!(c.effective_port(), Some(5432));
        assert_eq!(c.database, None);
    }

    #[test]
    fn postgres_url_no_host_and_query_user() {
        let c = p("postgresql:///mydb?user=bob&password=x%20y");
        assert_eq!(c.host, None);
        assert_eq!(c.effective_host(), "localhost");
        assert_eq!(c.database, some("mydb"));
        assert_eq!(c.user, some("bob"));
        assert_eq!(c.password, some("x y"));
    }

    #[test]
    fn postgres_url_ipv6() {
        let c = p("postgres://[::1]:5433/db");
        assert_eq!(c.host, some("::1"));
        assert_eq!(c.port, Some(5433));
    }

    #[test]
    fn mysql_url() {
        let c = p("mysql://root:pw@127.0.0.1:3307/app?ssl-mode=VERIFY_IDENTITY");
        assert_eq!(c.kind, DbKind::MySql);
        assert_eq!(c.port, Some(3307));
        assert_eq!(c.ssl, SslMode::VerifyFull);
        assert_eq!(c.database, some("app"));
    }

    #[test]
    fn mariadb_url_user_only() {
        let c = p("mariadb://app@dbhost/inventory");
        assert_eq!(c.kind, DbKind::MySql);
        assert_eq!(c.user, some("app"));
        assert_eq!(c.password, None);
        assert_eq!(c.effective_port(), Some(3306));
    }

    #[test]
    fn sqlserver_url_with_instance_and_encrypt() {
        let c = p(
            "sqlserver://sa:P%3Bw@sqlhost%5CSQLEXPRESS/Sales?encrypt=true&trustServerCertificate=true",
        );
        assert_eq!(c.kind, DbKind::MsSql);
        assert_eq!(c.host, some("sqlhost"));
        assert_eq!(c.instance, some("SQLEXPRESS"));
        assert_eq!(c.password, some("P;w"));
        assert_eq!(c.database, some("Sales"));
        assert_eq!(c.ssl, SslMode::Require);
        assert_eq!(c.effective_port(), None);
    }

    #[test]
    fn mssql_url_scheme() {
        let c = p("mssql://u:p@h:14330/db?encrypt=false");
        assert_eq!(c.kind, DbKind::MsSql);
        assert_eq!(c.port, Some(14330));
        assert_eq!(c.ssl, SslMode::Disable);
    }

    #[test]
    fn sqlite_url_forms() {
        assert_eq!(
            p("sqlite:///tmp/a.db").path,
            Some(PathBuf::from("/tmp/a.db"))
        );
        assert_eq!(p("sqlite://rel/a.db").path, Some(PathBuf::from("rel/a.db")));
        assert_eq!(
            p("sqlite:data.sqlite").path,
            Some(PathBuf::from("data.sqlite"))
        );
        assert_eq!(p("sqlite::memory:").path, Some(PathBuf::from(":memory:")));
        let c = p("file:///var/my%20db.sqlite3?mode=ro");
        assert_eq!(c.kind, DbKind::Sqlite);
        assert_eq!(c.path, Some(PathBuf::from("/var/my db.sqlite3")));
        assert!(c.read_only);
        assert!(!c.params.contains_key("mode"));
        assert_eq!(c.name, "my db.sqlite3");
        // any other mode stays a param
        let c = p("file:x.db?mode=rwc");
        assert!(!c.read_only);
        assert_eq!(c.params.get("mode"), Some(&"rwc".to_string()));
    }

    #[test]
    fn read_only_in_every_format() {
        for s in [
            "postgres://u@h/db?readonly=true",
            "postgres://u@h/db?read_only=1",
            "postgres://u@h/db?read-only=yes",
            "host=h dbname=db readonly=true",
            "mysql://u@h/db?readonly=1",
            "jdbc:mysql://h/db?readOnly=true",
            "jdbc:postgresql://h/db?readOnly=true",
            "jdbc:sqlserver://h;databaseName=db;readOnly=true",
            "sqlite:///tmp/a.db?mode=ro",
            "sqlite:///tmp/a.db?readonly=true",
            "Data Source=/tmp/a.db;Read Only=True",
            "Data Source=/tmp/a.db;Mode=ReadOnly",
            "Server=h,1433;Database=d;User Id=u;ApplicationIntent=ReadOnly",
            "Server=h,1433;Database=d;User Id=u;ReadOnly=true",
            "server=127.0.0.1;uid=root;database=test;readonly=yes",
        ] {
            let c = p(s);
            assert!(c.read_only, "{s}");
            assert!(
                !c.params.keys().any(|k| norm_key(k) == "readonly"),
                "{s}: {:?}",
                c.params
            );
        }
        for s in [
            "postgres://u@h/db",
            "postgres://u@h/db?readonly=false",
            "Server=h,1433;Database=d;ApplicationIntent=ReadWrite",
            "Data Source=/tmp/a.db;Mode=ReadWriteCreate",
        ] {
            assert!(!p(s).read_only, "{s}");
        }
        assert!(matches!(
            parse_connection_string("postgres://h/db?readonly=maybe"),
            Err(ParseError::Malformed(_))
        ));
        // ApplicationIntent still marks the semicolon form as SQL Server
        assert_eq!(
            p("Server=h;Database=d;ApplicationIntent=ReadOnly").kind,
            DbKind::MsSql
        );
    }

    #[test]
    fn sqlite_bare_paths() {
        assert_eq!(p("/home/me/app.db").kind, DbKind::Sqlite);
        assert_eq!(
            p("C:\\data\\app.SQLITE3").path,
            Some(PathBuf::from("C:\\data\\app.SQLITE3"))
        );
        assert_eq!(p("./x.db3").kind, DbKind::Sqlite);
        assert_eq!(p(":memory:").path, Some(PathBuf::from(":memory:")));
    }

    #[test]
    fn sqlite_hint_takes_any_path() {
        let c = parse_connection_string_with("/data/store", Some(DbKind::Sqlite)).unwrap();
        assert_eq!(c.path, Some(PathBuf::from("/data/store")));
        assert!(parse_connection_string("/data/store").is_err());
    }

    #[test]
    fn sqlite_requires_path() {
        assert_eq!(
            parse_connection_string("sqlite:"),
            Err(ParseError::Missing("path"))
        );
        assert_eq!(
            ConnectionConfig::new(DbKind::Sqlite).validate(),
            Err(ParseError::Missing("path"))
        );
    }

    #[test]
    fn libpq_keyvalue() {
        let c = p(
            "host=pg.local port=5433 dbname=app user=me password='a b' sslmode=verify-ca connect_timeout=5",
        );
        assert_eq!(c.kind, DbKind::Postgres);
        assert_eq!(c.host, some("pg.local"));
        assert_eq!(c.port, Some(5433));
        assert_eq!(c.database, some("app"));
        assert_eq!(c.ssl, SslMode::VerifyFull);
        assert_eq!(c.params.get("connect_timeout"), Some(&"5".to_string()));
        assert_eq!(c.password, some("a b"));
    }

    #[test]
    fn libpq_escapes_and_spacing() {
        let c = p("host = /var/run/postgresql  dbname = 'my db'  password='a\\'b'");
        assert_eq!(c.host, some("/var/run/postgresql"));
        assert_eq!(c.database, some("my db"));
        assert_eq!(c.password, some("a'b"));
    }

    #[test]
    fn ado_sqlserver_comma_port() {
        let c = p(
            "Server=tcp:sql.corp,1444;Initial Catalog=Orders;User ID=app;Password={p;w}}d};Encrypt=True;TrustServerCertificate=False",
        );
        assert_eq!(c.kind, DbKind::MsSql);
        assert_eq!(c.host, some("sql.corp"));
        assert_eq!(c.port, Some(1444));
        assert_eq!(c.database, some("Orders"));
        assert_eq!(c.user, some("app"));
        assert_eq!(c.password, some("p;w}d"));
        assert_eq!(c.ssl, SslMode::VerifyFull);
    }

    #[test]
    fn ado_sqlserver_instance_integrated() {
        let c = p("Data Source=.\\SQLEXPRESS;Database=Test;Integrated Security=SSPI;");
        assert_eq!(c.kind, DbKind::MsSql);
        assert_eq!(c.host, some("localhost"));
        assert_eq!(c.instance, some("SQLEXPRESS"));
        assert!(c.integrated_auth);
        assert_eq!(c.name, "Test @ localhost\\SQLEXPRESS");
    }

    #[test]
    fn ado_sqlserver_default_when_ambiguous() {
        let c = p("Server=myhost;Database=db;User Id=sa;Password=x");
        assert_eq!(c.kind, DbKind::MsSql);
    }

    #[test]
    fn ado_hint_resolves_ambiguity() {
        let c =
            parse_connection_string_with("Server=myhost;Database=db", Some(DbKind::MySql)).unwrap();
        assert_eq!(c.kind, DbKind::MySql);
        // ...but an explicit Driver beats the hint.
        let c = parse_connection_string_with(
            "Driver={PostgreSQL Unicode};Server=h",
            Some(DbKind::MySql),
        )
        .unwrap();
        assert_eq!(c.kind, DbKind::Postgres);
    }

    #[test]
    fn ado_sqlite_data_source() {
        let c = p("Data Source=C:\\apps\\local.db;Version=3;");
        assert_eq!(c.kind, DbKind::Sqlite);
        assert_eq!(c.path, Some(PathBuf::from("C:\\apps\\local.db")));
        assert_eq!(c.params.get("Version"), Some(&"3".to_string()));
    }

    #[test]
    fn ado_npgsql() {
        let c = p("Host=pg;Port=5432;Database=shop;Username=u;Password='p;x';SSL Mode=Require");
        assert_eq!(c.kind, DbKind::Postgres);
        assert_eq!(c.password, some("p;x"));
        assert_eq!(c.ssl, SslMode::Require);
    }

    #[test]
    fn ado_mysql_connector_net() {
        let c = p("server=127.0.0.1;uid=root;pwd=secret;database=test");
        assert_eq!(c.kind, DbKind::MySql);
        assert_eq!(c.user, some("root"));
        assert_eq!(c.password, some("secret"));
    }

    #[test]
    fn ado_port_hint() {
        assert_eq!(p("Server=h;Port=3306;Database=d").kind, DbKind::MySql);
        assert_eq!(p("Server=h;Port=5432;Database=d").kind, DbKind::Postgres);
    }

    #[test]
    fn ado_provider_hint() {
        let c = p("Provider=MSOLEDBSQL;Data Source=srv;Initial Catalog=x");
        assert_eq!(c.kind, DbKind::MsSql);
        assert!(!c.params.contains_key("Provider"));
    }

    #[test]
    fn odbc_driver_forms() {
        let c =
            p("Driver={ODBC Driver 18 for SQL Server};Server=srv,1435;Database=db;UID=u;PWD={a;b}");
        assert_eq!(c.kind, DbKind::MsSql);
        assert_eq!(c.port, Some(1435));
        assert_eq!(c.password, some("a;b"));
        let c = p(
            "DRIVER={MySQL ODBC 8.0 Unicode Driver};SERVER=localhost;DATABASE=x;USER=u;PASSWORD=p",
        );
        assert_eq!(c.kind, DbKind::MySql);
        assert_eq!(c.database, some("x"));
        let c = p("Driver=SQLite3;Database=/tmp/t.db");
        assert_eq!(c.kind, DbKind::Sqlite);
    }

    #[test]
    fn jdbc_postgres() {
        let c = p("jdbc:postgresql://pg:5432/app?user=u&password=p&ssl=true");
        assert_eq!(c.kind, DbKind::Postgres);
        assert_eq!(c.user, some("u"));
        assert_eq!(c.ssl, SslMode::Require);
    }

    #[test]
    fn jdbc_mysql_and_mariadb() {
        assert_eq!(p("jdbc:mysql://h:3306/d?user=r").kind, DbKind::MySql);
        let c = p("jdbc:mariadb://h/d?useSSL=false&serverTimezone=UTC");
        assert_eq!(c.kind, DbKind::MySql);
        assert_eq!(c.ssl, SslMode::Disable);
        assert_eq!(c.params.get("serverTimezone"), Some(&"UTC".to_string()));
    }

    #[test]
    fn jdbc_sqlserver() {
        let c = p(
            "jdbc:sqlserver://sql1\\INST:1500;databaseName=Hr;user=hr;password=x;encrypt=true;trustServerCertificate=true",
        );
        assert_eq!(c.kind, DbKind::MsSql);
        assert_eq!(c.host, some("sql1"));
        assert_eq!(c.instance, some("INST"));
        assert_eq!(c.port, Some(1500));
        assert_eq!(c.database, some("Hr"));
        assert_eq!(c.ssl, SslMode::Require);
    }

    #[test]
    fn jdbc_sqlserver_server_name_property() {
        let c = p(
            "jdbc:sqlserver://;serverName=srv;portNumber=1433;instanceName=X;integratedSecurity=true",
        );
        assert_eq!(c.host, some("srv"));
        assert_eq!(c.instance, some("X"));
        assert!(c.integrated_auth);
    }

    #[test]
    fn jdbc_sqlite() {
        assert_eq!(
            p("jdbc:sqlite:/data/x.db").path,
            Some(PathBuf::from("/data/x.db"))
        );
        assert_eq!(
            p("jdbc:sqlite::memory:").path,
            Some(PathBuf::from(":memory:"))
        );
    }

    #[test]
    fn errors() {
        assert_eq!(parse_connection_string("   "), Err(ParseError::Empty));
        assert_eq!(
            parse_connection_string("oracle://h/x"),
            Err(ParseError::UnknownScheme("oracle".into()))
        );
        assert_eq!(
            parse_connection_string("postgres://h:99999/x"),
            Err(ParseError::InvalidPort("99999".into()))
        );
        assert_eq!(
            parse_connection_string("hello world"),
            Err(ParseError::UnknownFormat)
        );
        assert!(matches!(
            parse_connection_string("Server=h;Password={oops"),
            Err(ParseError::Malformed(_))
        ));
        assert!(matches!(
            parse_connection_string("postgres://h/%zz"),
            Err(ParseError::Malformed(_))
        ));
        assert!(matches!(
            parse_connection_string("Server=h;Encrypt=maybe"),
            Err(ParseError::Malformed(_))
        ));
        assert!(matches!(
            parse_connection_string("jdbc:oracle:thin:@h"),
            Err(ParseError::UnknownScheme(_))
        ));
    }

    #[test]
    fn validate_rules() {
        let mut c = ConnectionConfig::new(DbKind::Postgres);
        assert_eq!(c.validate(), Ok(()));
        c.instance = some("X");
        assert!(c.validate().is_err());
        c.instance = None;
        c.port = Some(0);
        assert!(c.validate().is_err());
    }

    #[test]
    fn to_url_round_trips() {
        for s in [
            "postgres://alice:p%40ss@db:6543/shop?sslmode=require&application_name=dbx",
            "mysql://root@h/app?ssl-mode=DISABLED",
            "sqlserver://sa:x@sqlhost%5CSQLEXPRESS/Sales?encrypt=true&trustServerCertificate=true",
            "sqlite:///tmp/a%20b.db?mode=ro",
            "sqlite:rel.db",
            "postgres://u@h/db?readonly=true",
            "sqlserver://sa@h:1433/db?ApplicationIntent=ReadOnly",
            "mysql://u@h/db?read-only=1&connect_timeout=3",
        ] {
            let c = p(s);
            let back = p(&c.to_url(false));
            assert_eq!(
                ConnectionConfig {
                    name: c.name.clone(),
                    ..back
                },
                c,
                "{s}"
            );
        }
    }

    #[test]
    fn to_url_masks_password() {
        let c = p("postgres://u:secret@h/db");
        assert_eq!(c.to_url(true), "postgres://u:***@h/db");
        assert!(c.to_url(false).contains("secret"));
    }

    #[test]
    fn serde_round_trip() {
        let c = p("Server=h,1444;Database=d;User Id=u;Password=p;TrustServerCertificate=true");
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"kind\":\"mssql\""));
        let back: ConnectionConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
        // `read_only` is optional in saved JSON and written only when set
        assert!(!json.contains("read_only"));
        let ro = p("postgres://u@h/db?readonly=true");
        let json = serde_json::to_string(&ro).unwrap();
        assert!(json.contains("\"read_only\":true"));
        assert_eq!(serde_json::from_str::<ConnectionConfig>(&json).unwrap(), ro);
    }
}
