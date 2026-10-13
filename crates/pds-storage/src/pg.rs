//! A client of the Postgres wire protocol, for the shared tier
//! (`docs/SPEC.md` §8.4, §21.2).
//!
//! It does what the shared tier needs and no more: a blocking socket, the
//! SCRAM-SHA-256 or the clear-text password exchange, and statements with
//! parameters in the text format. Text parameters and text results give
//! the server the typing that SQLite has: a value takes the type of the
//! column it meets. The connection has no TLS: the server reaches its
//! database on a private network.

use std::io::{BufReader, BufWriter, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use data_encoding::{BASE64, HEXLOWER};
use sha2::{Digest, Sha256};

use crate::sql::Value;

/// Why a statement or a connection failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PgError {
    /// The socket failed, or the server sent what the client cannot read.
    #[error("postgres connection: {0}")]
    Connection(String),
    /// The server refused a statement.
    #[error("postgres {code}: {message}")]
    Server {
        /// The SQLSTATE.
        code: String,
        /// The message of the server.
        message: String,
    },
}

impl PgError {
    /// Whether the statement broke a unique constraint or a primary key.
    pub fn is_unique_violation(&self) -> bool {
        matches!(self, Self::Server { code, .. } if code == "23505")
    }
}

fn broken(what: impl std::fmt::Display) -> PgError {
    PgError::Connection(what.to_string())
}

/// Where a database is and who connects.
#[derive(Clone, PartialEq, Eq)]
pub struct PgTarget {
    host: String,
    port: u16,
    user: String,
    password: String,
    database: String,
}

impl std::fmt::Debug for PgTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PgTarget")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("database", &self.database)
            .finish_non_exhaustive()
    }
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let hex = text.get(at + 1..at + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(out).ok()
}

impl PgTarget {
    /// Reads `postgres://user:password@host:port/database`. A query is
    /// refused unless it is `sslmode=disable`: the client has no TLS.
    pub fn parse(url: &str) -> Result<Self, &'static str> {
        let rest = url
            .strip_prefix("postgres://")
            .or_else(|| url.strip_prefix("postgresql://"))
            .ok_or("expected a URL that starts with postgres://")?;
        let (rest, query) = rest.split_once('?').unwrap_or((rest, ""));
        if !matches!(query, "" | "sslmode=disable") {
            return Err("the only option the client takes is sslmode=disable: it has no TLS");
        }
        let (credentials, place) = rest
            .rsplit_once('@')
            .ok_or("expected a user and a password before the host")?;
        let (user, password) = credentials.split_once(':').unwrap_or((credentials, ""));
        let (address, database) = place.split_once('/').ok_or("expected a database name")?;
        let (host, port) = match address.rsplit_once(':') {
            Some((host, port)) => (host, port.parse().map_err(|_| "the port is not a number")?),
            None => (address, 5432),
        };
        let decoded = |text: &str| percent_decode(text).ok_or("a percent escape is not valid");
        if user.is_empty() || host.is_empty() || database.is_empty() {
            return Err("expected a user, a host and a database name");
        }
        Ok(Self {
            host: host.to_owned(),
            port,
            user: decoded(user)?,
            password: decoded(password)?,
            database: decoded(database)?,
        })
    }
}

/// HMAC-SHA-256 (RFC 2104).
fn hmac(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut block = [0_u8; 64];
    if key.len() > 64 {
        block[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let pad = |with: u8| -> Vec<u8> { block.iter().map(|byte| byte ^ with).collect() };
    let inner = Sha256::new()
        .chain_update(pad(0x36))
        .chain_update(message)
        .finalize();
    Sha256::new()
        .chain_update(pad(0x5c))
        .chain_update(inner)
        .finalize()
        .into()
}

/// PBKDF2 with HMAC-SHA-256 and one block of output (RFC 8018).
fn pbkdf2(password: &[u8], salt: &[u8], rounds: u32) -> [u8; 32] {
    let mut first = salt.to_vec();
    first.extend_from_slice(&1_u32.to_be_bytes());
    let mut round = hmac(password, &first);
    let mut out = round;
    for _ in 1..rounds {
        round = hmac(password, &round);
        for (byte, with) in out.iter_mut().zip(round) {
            *byte ^= with;
        }
    }
    out
}

/// The client's last message of SCRAM-SHA-256 (RFC 7677) and the
/// signature the server must answer with.
fn scram_final(
    password: &str,
    first_bare: &str,
    server_first: &str,
) -> Result<(String, [u8; 32]), PgError> {
    let field = |name: &str| {
        server_first
            .split(',')
            .find_map(|part| part.strip_prefix(name))
            .ok_or_else(|| broken("the SCRAM message of the server lacks a field"))
    };
    let nonce = field("r=")?;
    let salt = BASE64
        .decode(field("s=")?.as_bytes())
        .map_err(|_| broken("the SCRAM salt is not base64"))?;
    let rounds: u32 = field("i=")?
        .parse()
        .map_err(|_| broken("the SCRAM iteration count is not a number"))?;
    let client_nonce = first_bare.rsplit("r=").next().unwrap_or_default();
    if !nonce.starts_with(client_nonce) || rounds == 0 {
        return Err(broken("the SCRAM nonce of the server does not extend the client's"));
    }
    let salted = pbkdf2(password.as_bytes(), &salt, rounds);
    let client_key = hmac(&salted, b"Client Key");
    let stored_key = Sha256::digest(client_key);
    let without_proof = format!("c=biws,r={nonce}");
    let auth = format!("{first_bare},{server_first},{without_proof}");
    let signature = hmac(&stored_key, auth.as_bytes());
    let proof: Vec<u8> = client_key
        .iter()
        .zip(signature)
        .map(|(key, sign)| key ^ sign)
        .collect();
    let server = hmac(&hmac(&salted, b"Server Key"), auth.as_bytes());
    Ok((
        format!("{without_proof},p={}", BASE64.encode(&proof)),
        server,
    ))
}

/// What a statement gave: the number of rows it changed, and its rows.
pub type Outcome = (u64, Vec<Vec<Value>>);

/// A message of the server: its type and its body.
struct Message {
    kind: u8,
    body: Vec<u8>,
}

/// Reads the fields of a message body in order.
struct Fields<'a>(&'a [u8]);

impl Fields<'_> {
    fn take(&mut self, len: usize) -> Result<&[u8], PgError> {
        if self.0.len() < len {
            return Err(broken("a message of the server is short"));
        }
        let (head, rest) = self.0.split_at(len);
        self.0 = rest;
        Ok(head)
    }

    fn i16(&mut self) -> Result<i16, PgError> {
        let bytes = self.take(2)?;
        Ok(i16::from_be_bytes([bytes[0], bytes[1]]))
    }

    fn i32(&mut self) -> Result<i32, PgError> {
        let bytes = self.take(4)?;
        Ok(i32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn text(&mut self) -> Result<String, PgError> {
        let end = self
            .0
            .iter()
            .position(|byte| *byte == 0)
            .ok_or_else(|| broken("a string of the server has no end"))?;
        let text = String::from_utf8_lossy(&self.0[..end]).into_owned();
        self.0 = &self.0[end + 1..];
        Ok(text)
    }
}

fn server_error(body: &[u8]) -> PgError {
    let mut fields = Fields(body);
    let (mut code, mut message) = (String::new(), String::new());
    while let Ok(kind) = fields.take(1).map(|kind| kind[0]) {
        if kind == 0 {
            break;
        }
        let Ok(value) = fields.text() else { break };
        match kind {
            b'C' => code = value,
            b'M' => message = value,
            _ => {}
        }
    }
    PgError::Server { code, message }
}

/// A value of the server in the text format, by the type of its column.
fn decode(oid: i32, text: &[u8]) -> Result<Value, PgError> {
    let as_text = || String::from_utf8(text.to_vec()).map_err(|_| broken("a value is not UTF-8"));
    Ok(match oid {
        // bool
        16 => Value::Integer(i64::from(text == b"t")),
        // int8, int2, int4, oid
        20 | 21 | 23 | 26 => Value::Integer(
            as_text()?
                .parse()
                .map_err(|_| broken("an integer of the server does not parse"))?,
        ),
        // bytea, as `\x` and hexadecimal digits
        17 => {
            let hex = text
                .strip_prefix(b"\\x")
                .ok_or_else(|| broken("a bytea value is not in the hex format"))?;
            Value::Blob(
                HEXLOWER
                    .decode(hex)
                    .map_err(|_| broken("a bytea value is not hexadecimal"))?,
            )
        }
        _ => Value::Text(as_text()?),
    })
}

/// A parameter in the text format. `None` is NULL.
fn encode(value: &Value) -> Option<Vec<u8>> {
    match value {
        Value::Null => None,
        Value::Integer(number) => Some(number.to_string().into_bytes()),
        Value::Text(text) => Some(text.clone().into_bytes()),
        Value::Blob(bytes) => Some(format!("\\x{}", HEXLOWER.encode(bytes)).into_bytes()),
    }
}

/// The rows a command changed, from its tag: `INSERT 0 2`, `UPDATE 3`.
fn changed(tag: &str) -> u64 {
    tag.rsplit(' ')
        .next()
        .and_then(|count| count.parse().ok())
        .unwrap_or(0)
}

/// One connection to a Postgres server.
pub struct PgClient {
    reader: BufReader<TcpStream>,
    writer: BufWriter<TcpStream>,
}

impl std::fmt::Debug for PgClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PgClient").finish_non_exhaustive()
    }
}

fn frame(kind: u8, body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len() + 5);
    out.push(kind);
    out.extend_from_slice(&(u32::try_from(body.len() + 4).unwrap_or(u32::MAX)).to_be_bytes());
    out.extend_from_slice(body);
    out
}

fn cstring(out: &mut Vec<u8>, text: &str) {
    out.extend_from_slice(text.as_bytes());
    out.push(0);
}

impl PgClient {
    /// Connects and signs in. A statement that waits longer than
    /// `timeout` for the server fails.
    pub fn connect(target: &PgTarget, timeout: Duration) -> Result<Self, PgError> {
        let stream = TcpStream::connect((target.host.as_str(), target.port)).map_err(broken)?;
        stream.set_nodelay(true).map_err(broken)?;
        stream.set_read_timeout(Some(timeout)).map_err(broken)?;
        stream.set_write_timeout(Some(timeout)).map_err(broken)?;
        let mut client = Self {
            reader: BufReader::new(stream.try_clone().map_err(broken)?),
            writer: BufWriter::new(stream),
        };
        let mut body = 196_608_i32.to_be_bytes().to_vec();
        for (name, value) in [
            ("user", target.user.as_str()),
            ("database", target.database.as_str()),
            ("client_encoding", "UTF8"),
            ("application_name", "linkjar-pds"),
        ] {
            cstring(&mut body, name);
            cstring(&mut body, value);
        }
        body.push(0);
        let len = u32::try_from(body.len() + 4).unwrap_or(u32::MAX);
        client.send(&[len.to_be_bytes().as_slice(), &body].concat())?;
        client.sign_in(target)?;
        Ok(client)
    }

    fn send(&mut self, bytes: &[u8]) -> Result<(), PgError> {
        self.writer.write_all(bytes).map_err(broken)?;
        self.writer.flush().map_err(broken)
    }

    fn read(&mut self) -> Result<Message, PgError> {
        let mut head = [0_u8; 5];
        self.reader.read_exact(&mut head).map_err(broken)?;
        let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
        if !(4..=1 << 30).contains(&len) {
            return Err(broken("a message of the server has an impossible length"));
        }
        let mut body = vec![0_u8; len - 4];
        self.reader.read_exact(&mut body).map_err(broken)?;
        Ok(Message {
            kind: head[0],
            body,
        })
    }

    fn sign_in(&mut self, target: &PgTarget) -> Result<(), PgError> {
        let mut nonce = [0_u8; 18];
        rand::fill(&mut nonce[..]);
        let first_bare = format!("n=,r={}", BASE64.encode(&nonce));
        let mut expected = None;
        loop {
            let message = self.read()?;
            match message.kind {
                b'E' => return Err(server_error(&message.body)),
                b'Z' => return Ok(()),
                b'R' => {
                    let mut fields = Fields(&message.body);
                    match fields.i32()? {
                        0 => {}
                        3 => {
                            let mut body = Vec::new();
                            cstring(&mut body, &target.password);
                            self.send(&frame(b'p', &body))?;
                        }
                        10 => {
                            let mut body = Vec::new();
                            cstring(&mut body, "SCRAM-SHA-256");
                            let first = format!("n,,{first_bare}");
                            let len = i32::try_from(first.len()).unwrap_or(i32::MAX);
                            body.extend_from_slice(&len.to_be_bytes());
                            body.extend_from_slice(first.as_bytes());
                            self.send(&frame(b'p', &body))?;
                        }
                        11 => {
                            let server_first = String::from_utf8_lossy(fields.0).into_owned();
                            let (last, signature) =
                                scram_final(&target.password, &first_bare, &server_first)?;
                            expected = Some(signature);
                            self.send(&frame(b'p', last.as_bytes()))?;
                        }
                        12 => {
                            let last = String::from_utf8_lossy(fields.0).into_owned();
                            let given = last
                                .strip_prefix("v=")
                                .and_then(|value| BASE64.decode(value.as_bytes()).ok());
                            if given.as_deref() != expected.as_ref().map(<[u8; 32]>::as_slice) {
                                return Err(broken("the server did not prove that it knows the password"));
                            }
                        }
                        _ => {
                            return Err(broken(
                                "the server asks for a sign-in the client does not have; use scram-sha-256",
                            ));
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Reads the answers of one statement up to `ReadyForQuery`.
    fn collect(&mut self) -> Result<Outcome, PgError> {
        let mut types: Vec<i32> = Vec::new();
        let mut rows = Vec::new();
        let mut count = 0;
        let mut failure = None;
        loop {
            let message = self.read()?;
            let mut fields = Fields(&message.body);
            match message.kind {
                b'E' => failure = Some(server_error(&message.body)),
                b'T' => {
                    types.clear();
                    for _ in 0..fields.i16()? {
                        fields.text()?;
                        fields.take(6)?;
                        types.push(fields.i32()?);
                        fields.take(8)?;
                    }
                }
                b'D' => {
                    let mut row = Vec::with_capacity(types.len());
                    for index in 0..usize::try_from(fields.i16()?).unwrap_or(0) {
                        let len = fields.i32()?;
                        if len < 0 {
                            row.push(Value::Null);
                        } else {
                            let oid = types.get(index).copied().unwrap_or(25);
                            let len = usize::try_from(len).unwrap_or(0);
                            row.push(decode(oid, fields.take(len)?)?);
                        }
                    }
                    rows.push(row);
                }
                b'C' => count = changed(&fields.text()?),
                b'Z' => {
                    return match failure {
                        Some(error) => Err(error),
                        None => Ok((count, rows)),
                    };
                }
                _ => {}
            }
        }
    }

    /// Runs one statement with `$1`, `$2`, … for its parameters.
    pub fn query(&mut self, sql: &str, params: &[Value]) -> Result<Outcome, PgError> {
        let mut parse = vec![0];
        cstring(&mut parse, sql);
        parse.extend_from_slice(&0_i16.to_be_bytes());
        let mut bind = vec![0, 0];
        bind.extend_from_slice(&0_i16.to_be_bytes());
        bind.extend_from_slice(&i16::try_from(params.len()).unwrap_or(i16::MAX).to_be_bytes());
        for value in params {
            match encode(value) {
                Some(bytes) => {
                    let len = i32::try_from(bytes.len()).unwrap_or(i32::MAX);
                    bind.extend_from_slice(&len.to_be_bytes());
                    bind.extend_from_slice(&bytes);
                }
                None => bind.extend_from_slice(&(-1_i32).to_be_bytes()),
            }
        }
        bind.extend_from_slice(&0_i16.to_be_bytes());
        let mut execute = vec![0];
        execute.extend_from_slice(&0_i32.to_be_bytes());
        let batch = [
            frame(b'P', &parse),
            frame(b'B', &bind),
            frame(b'D', b"P\0"),
            frame(b'E', &execute),
            frame(b'S', &[]),
        ]
        .concat();
        self.send(&batch)?;
        self.collect()
    }

    /// Runs statements without parameters, separated by semicolons.
    pub fn batch(&mut self, sql: &str) -> Result<(), PgError> {
        let mut body = Vec::new();
        cstring(&mut body, sql);
        self.send(&frame(b'Q', &body))?;
        self.collect().map(|_| ())
    }
}

impl Drop for PgClient {
    fn drop(&mut self) {
        let _ = self.send(&frame(b'X', &[]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_and_pbkdf2_give_the_published_values() {
        // RFC 4231, test case 2.
        assert_eq!(
            HEXLOWER.encode(&hmac(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // RFC 7914 §11: PBKDF2-HMAC-SHA-256, "passwd" and "salt", 1 round.
        assert_eq!(
            HEXLOWER.encode(&pbkdf2(b"passwd", b"salt", 1)),
            "55ac046e56e3089fec1691c22544b605f94185216dde0465e68b9d57c20dacbc"
        );
    }

    #[test]
    fn scram_gives_the_proof_of_the_rfc() {
        // RFC 7677 §3.
        let first_bare = "n=user,r=rOprNGfwEbeRWgbNEkqO";
        let server_first = "r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096";
        let (last, server) = scram_final("pencil", first_bare, server_first).unwrap();
        assert_eq!(
            last,
            "c=biws,r=rOprNGfwEbeRWgbNEkqO%hvYDpWUa2RaTCAfuxFIlj)hNlF$k0,p=dHzbZapWIk4jUhN+Ute9ytag9zjfMHgsqmmiz7AndVQ="
        );
        assert_eq!(
            BASE64.encode(&server),
            "6rriTRBi23WpRR/wtup+mMhUZUn/dB5nLTJRsjl95G4="
        );
        // A server that answers with another nonce is refused.
        assert!(scram_final("pencil", first_bare, "r=other,s=W22ZaJ0SNY7soEsUEjb6gQ==,i=4096").is_err());
    }

    #[test]
    fn a_target_is_read_from_a_url() {
        let target = PgTarget::parse("postgres://pds:p%40ss@db.internal:6543/linkjar").unwrap();
        assert_eq!(
            (target.host.as_str(), target.port, target.user.as_str()),
            ("db.internal", 6543, "pds")
        );
        assert_eq!((target.password.as_str(), target.database.as_str()), ("p@ss", "linkjar"));
        assert_eq!(PgTarget::parse("postgresql://u@h/d?sslmode=disable").unwrap().port, 5432);
        assert!(!format!("{target:?}").contains("p@ss"));
        for bad in [
            "mysql://u:p@h/d",
            "postgres://h/d",
            "postgres://u:p@h",
            "postgres://u:p@h:x/d",
            "postgres://u:p@h/d?sslmode=require",
        ] {
            assert!(PgTarget::parse(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn values_cross_the_text_format() {
        assert_eq!(decode(16, b"t").unwrap(), Value::Integer(1));
        assert_eq!(decode(20, b"-42").unwrap(), Value::Integer(-42));
        assert_eq!(decode(17, b"\\x00ff").unwrap(), Value::Blob(vec![0, 255]));
        assert_eq!(decode(1043, b"text").unwrap(), Value::Text("text".to_owned()));
        assert_eq!(encode(&Value::Blob(vec![0, 255])).unwrap(), b"\\x00ff");
        assert_eq!(encode(&Value::Null), None);
        assert_eq!(changed("INSERT 0 2"), 2);
        assert_eq!(changed("UPDATE 3"), 3);
        assert_eq!(changed("CREATE TABLE"), 0);
    }
}
