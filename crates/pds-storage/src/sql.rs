//! One way to run SQL on the two backends of the shared tier
//! (`docs/SPEC.md` §8.4): SQLite and Postgres.
//!
//! The statements of the shared tier are written once, in the dialect
//! both servers take, with `?1` or `:name` for a parameter. This module
//! binds the parameters, runs the statement, and gives the rows as
//! values. The names follow `rusqlite`, which the code used before the
//! second backend came.

use std::cell::{Cell, RefCell};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use crate::pg::{PgClient, PgError, PgTarget};

/// A value of a column or of a parameter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    /// NULL.
    Null,
    /// An integer. A boolean is 0 or 1.
    Integer(i64),
    /// Text.
    Text(String),
    /// Bytes.
    Blob(Vec<u8>),
}

impl From<String> for Value {
    fn from(text: String) -> Self {
        Self::Text(text)
    }
}

impl From<i64> for Value {
    fn from(number: i64) -> Self {
        Self::Integer(number)
    }
}

/// A statement failed.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// SQLite reported an error.
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// Postgres reported an error.
    #[error("{0}")]
    Postgres(#[from] PgError),
    /// A query for one row found none.
    #[error("the query returned no row")]
    NoRows,
    /// A column holds a value of another type than the caller reads.
    #[error("column {index}: expected {expected}")]
    Type {
        /// The column, from 0.
        index: usize,
        /// What the caller reads.
        expected: &'static str,
    },
}

/// The result of a statement.
pub type Result<T> = std::result::Result<T, Error>;

/// Turns "no row" into `None`.
pub trait OptionalExtension<T> {
    /// `Ok(None)` when the query found no row.
    fn optional(self) -> Result<Option<T>>;
}

impl<T> OptionalExtension<T> for Result<T> {
    fn optional(self) -> Result<Option<T>> {
        match self {
            Ok(value) => Ok(Some(value)),
            Err(Error::NoRows) => Ok(None),
            Err(error) => Err(error),
        }
    }
}

/// A Rust value as a parameter.
pub trait ToValue {
    /// The value to bind.
    fn to_value(&self) -> Value;
}

impl ToValue for Value {
    fn to_value(&self) -> Value {
        self.clone()
    }
}

impl ToValue for str {
    fn to_value(&self) -> Value {
        Value::Text(self.to_owned())
    }
}

impl ToValue for String {
    fn to_value(&self) -> Value {
        Value::Text(self.clone())
    }
}

impl ToValue for [u8] {
    fn to_value(&self) -> Value {
        Value::Blob(self.to_vec())
    }
}

impl ToValue for Vec<u8> {
    fn to_value(&self) -> Value {
        Value::Blob(self.clone())
    }
}

impl ToValue for bool {
    fn to_value(&self) -> Value {
        Value::Integer(i64::from(*self))
    }
}

macro_rules! integer_to_value {
    ($($kind:ty),*) => {$(
        impl ToValue for $kind {
            fn to_value(&self) -> Value {
                Value::Integer(i64::try_from(*self).unwrap_or(i64::MAX))
            }
        }
    )*};
}
integer_to_value!(i64, i32, u32, u64, usize);

impl<T: ToValue + ?Sized> ToValue for &T {
    fn to_value(&self) -> Value {
        (**self).to_value()
    }
}

impl<T: ToValue> ToValue for Option<T> {
    fn to_value(&self) -> Value {
        self.as_ref().map_or(Value::Null, ToValue::to_value)
    }
}

/// A value of a column as a Rust value.
pub trait FromValue: Sized {
    /// What the type is called in an error.
    const EXPECTED: &'static str;
    /// The Rust value, or `None` when the column holds another type.
    fn from_value(value: Value) -> Option<Self>;
}

impl FromValue for i64 {
    const EXPECTED: &'static str = "an integer";
    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::Integer(number) => Some(number),
            _ => None,
        }
    }
}

impl FromValue for u32 {
    const EXPECTED: &'static str = "an integer from 0";
    fn from_value(value: Value) -> Option<Self> {
        i64::from_value(value).and_then(|number| Self::try_from(number).ok())
    }
}

impl FromValue for u64 {
    const EXPECTED: &'static str = "an integer from 0";
    fn from_value(value: Value) -> Option<Self> {
        i64::from_value(value).and_then(|number| Self::try_from(number).ok())
    }
}

impl FromValue for bool {
    const EXPECTED: &'static str = "a boolean";
    fn from_value(value: Value) -> Option<Self> {
        i64::from_value(value).map(|number| number != 0)
    }
}

impl FromValue for String {
    const EXPECTED: &'static str = "text";
    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::Text(text) => Some(text),
            _ => None,
        }
    }
}

impl FromValue for Vec<u8> {
    const EXPECTED: &'static str = "bytes";
    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::Blob(bytes) => Some(bytes),
            // SQLite gives the bytes of text for a column without a type.
            Value::Text(text) => Some(text.into_bytes()),
            _ => None,
        }
    }
}

impl<T: FromValue> FromValue for Option<T> {
    const EXPECTED: &'static str = T::EXPECTED;
    fn from_value(value: Value) -> Option<Self> {
        match value {
            Value::Null => Some(None),
            value => T::from_value(value).map(Some),
        }
    }
}

/// The parameters of a statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Params {
    /// For `?1`, `?2`, … in order.
    Positional(Vec<Value>),
    /// For `:name`.
    Named(Vec<(&'static str, Value)>),
}

/// What a statement takes as its parameters.
pub trait IntoParams {
    /// The parameters.
    fn into_params(self) -> Params;
}

impl IntoParams for Params {
    fn into_params(self) -> Params {
        self
    }
}

impl IntoParams for [&dyn ToValue; 0] {
    fn into_params(self) -> Params {
        Params::Positional(Vec::new())
    }
}

macro_rules! array_into_params {
    ($($len:literal),*) => {$(
        impl<T: ToValue> IntoParams for [T; $len] {
            fn into_params(self) -> Params {
                Params::Positional(self.iter().map(ToValue::to_value).collect())
            }
        }
    )*};
}
array_into_params!(1, 2, 3, 4, 5, 6, 7, 8);

/// The values of an iterator as positional parameters.
pub fn params_from_iter(values: impl IntoIterator<Item = Value>) -> Params {
    Params::Positional(values.into_iter().collect())
}

/// Positional parameters of mixed types.
#[macro_export]
macro_rules! params {
    ($($value:expr),* $(,)?) => {
        $crate::sql::Params::Positional(vec![$($crate::sql::ToValue::to_value(&$value)),*])
    };
}

/// Named parameters: `named_params! { ":did": did }`.
#[macro_export]
macro_rules! named_params {
    ($($name:literal: $value:expr),* $(,)?) => {
        $crate::sql::Params::Named(vec![$(($name, $crate::sql::ToValue::to_value(&$value))),*])
    };
}

pub use {named_params, params};

/// A row of a result.
#[derive(Debug, Clone, Copy)]
pub struct Row<'a> {
    values: &'a [Value],
}

impl Row<'_> {
    /// The value of column `index`, from 0.
    pub fn get<T: FromValue>(&self, index: usize) -> Result<T> {
        let failed = Error::Type {
            index,
            expected: T::EXPECTED,
        };
        match self.values.get(index) {
            Some(value) => T::from_value(value.clone()).ok_or(failed),
            None => Err(failed),
        }
    }
}

/// Whether `byte` can be part of a parameter name.
const fn is_name(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// A statement in the dialect of Postgres: `?1` and `:name` become `$n`.
/// Gives the values in the order of the `$n`. Text in quotes is left as
/// it is.
fn for_postgres(sql: &str, params: Params) -> (String, Vec<Value>) {
    let bytes = sql.as_bytes();
    let mut out = String::with_capacity(sql.len() + 8);
    let mut names: Vec<&str> = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        if byte == b'\'' || byte == b'"' {
            let end = bytes[at + 1..]
                .iter()
                .position(|other| *other == byte)
                .map_or(bytes.len(), |offset| at + 1 + offset + 1);
            out.push_str(&sql[at..end]);
            at = end;
        } else if byte == b'?' && bytes.get(at + 1).is_some_and(u8::is_ascii_digit) {
            out.push('$');
            at += 1;
        } else if byte == b':'
            && bytes.get(at + 1).is_some_and(u8::is_ascii_alphabetic)
            && (at == 0 || bytes[at - 1] != b':')
        {
            let end = bytes[at + 1..]
                .iter()
                .position(|other| !is_name(*other))
                .map_or(bytes.len(), |offset| at + 1 + offset);
            let name = &sql[at..end];
            let index = names
                .iter()
                .position(|known| *known == name)
                .unwrap_or_else(|| {
                    names.push(name);
                    names.len() - 1
                });
            out.push('$');
            out.push_str(&(index + 1).to_string());
            at = end;
        } else {
            // A multi-byte character goes through byte by byte.
            let end = (at + 1..=bytes.len())
                .find(|end| sql.is_char_boundary(*end))
                .unwrap_or(bytes.len());
            out.push_str(&sql[at..end]);
            at = end;
        }
    }
    let values = match params {
        Params::Positional(values) => values,
        Params::Named(mut given) => names
            .iter()
            .map(|name| {
                given
                    .iter_mut()
                    .find(|(known, _)| known == name)
                    .map_or(Value::Null, |(_, value)| {
                        std::mem::replace(value, Value::Null)
                    })
            })
            .collect(),
    };
    (out, values)
}

/// A statement that creates a table or an index, in the types of
/// Postgres. Text compares by its bytes, as in SQLite.
pub fn ddl_for_postgres(statement: &str) -> String {
    let mut out = String::new();
    let mut quoted = false;
    for (index, part) in statement.split('"').enumerate() {
        if index > 0 {
            out.push('"');
        }
        if quoted {
            out.push_str(part);
        } else {
            let lower = part
                .replace(
                    " integer primary key autoincrement",
                    " bigserial primary key",
                )
                .replace(" integer", " bigint")
                .replace(" blob", " bytea")
                .replace(" varchar(255)", " varchar")
                .replace(" varchar", " varchar collate \"C\"")
                .replace(" text", " text collate \"C\"");
            out.push_str(&lower);
        }
        quoted = !quoted;
    }
    out
}

impl From<rusqlite::types::ValueRef<'_>> for Value {
    fn from(value: rusqlite::types::ValueRef<'_>) -> Self {
        use rusqlite::types::ValueRef;
        match value {
            ValueRef::Null => Self::Null,
            ValueRef::Integer(number) => Self::Integer(number),
            // The shared tier has no column of real numbers.
            #[allow(clippy::cast_possible_truncation)]
            ValueRef::Real(number) => Self::Integer(number as i64),
            ValueRef::Text(text) => Self::Text(String::from_utf8_lossy(text).into_owned()),
            ValueRef::Blob(bytes) => Self::Blob(bytes.to_vec()),
        }
    }
}

impl rusqlite::ToSql for Value {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        use rusqlite::types::{ToSqlOutput, ValueRef};
        Ok(ToSqlOutput::Borrowed(match self {
            Self::Null => ValueRef::Null,
            Self::Integer(number) => ValueRef::Integer(*number),
            Self::Text(text) => ValueRef::Text(text.as_bytes()),
            Self::Blob(bytes) => ValueRef::Blob(bytes),
        }))
    }
}

/// The rows a statement changed, and the rows it gave.
type Outcome = (usize, Vec<Vec<Value>>);

fn run_sqlite(conn: &rusqlite::Connection, sql: &str, params: &Params) -> Result<Outcome> {
    let mut statement = conn.prepare_cached(sql)?;
    match params {
        Params::Positional(values) => {
            for (index, value) in values.iter().enumerate() {
                statement.raw_bind_parameter(index + 1, value)?;
            }
        }
        Params::Named(values) => {
            for (name, value) in values {
                if let Some(index) = statement.parameter_index(name)? {
                    statement.raw_bind_parameter(index, value)?;
                }
            }
        }
    }
    let columns = statement.column_count();
    if columns == 0 {
        return Ok((statement.raw_execute()?, Vec::new()));
    }
    let mut found = Vec::new();
    let mut rows = statement.raw_query();
    while let Some(row) = rows.next()? {
        let mut values = Vec::with_capacity(columns);
        for index in 0..columns {
            values.push(Value::from(row.get_ref(index)?));
        }
        found.push(values);
    }
    Ok((found.len(), found))
}

/// A Postgres database of the shared tier: where it is, and the schema
/// that holds the tables of one database of the data directory.
#[derive(Debug)]
pub struct PgPool {
    target: PgTarget,
    schema: String,
    idle: Mutex<Vec<PgClient>>,
}

/// How long a statement waits for the server.
const PG_TIMEOUT: Duration = Duration::from_secs(30);

/// The most idle connections a pool keeps.
const PG_IDLE: usize = 8;

impl PgPool {
    /// A pool for `schema` of the database at `target`. A schema name has
    /// lower-case letters, digits and underscores.
    pub fn new(target: PgTarget, schema: &str) -> Arc<Self> {
        let schema = schema
            .chars()
            .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == '_')
            .collect();
        Arc::new(Self {
            target,
            schema,
            idle: Mutex::new(Vec::new()),
        })
    }

    /// The schema.
    pub fn schema(&self) -> &str {
        &self.schema
    }

    /// Creates the schema when it is not there.
    pub fn create_schema(self: &Arc<Self>) -> Result<()> {
        let mut client = PgClient::connect(&self.target, PG_TIMEOUT)?;
        client.batch(&format!(r#"create schema if not exists "{}""#, self.schema))?;
        Ok(())
    }

    /// Whether the schema has a table.
    pub fn is_created(self: &Arc<Self>) -> Result<bool> {
        let mut client = PgClient::connect(&self.target, PG_TIMEOUT)?;
        let (_, rows) = client.query(
            "select count(*) from information_schema.tables where table_schema = $1",
            &[Value::Text(self.schema.clone())],
        )?;
        Ok(rows.first().and_then(|row| row.first()) != Some(&Value::Integer(0)))
    }

    /// A connection that reads and writes the schema.
    pub fn connect(self: &Arc<Self>) -> Result<Connection> {
        let idle = self
            .idle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop();
        let client = if let Some(client) = idle {
            client
        } else {
            let mut client = PgClient::connect(&self.target, PG_TIMEOUT)?;
            client.batch(&format!(r#"set search_path to "{}""#, self.schema))?;
            client
        };
        Ok(Connection {
            inner: Inner::Postgres {
                client: RefCell::new(Some(client)),
                pool: Arc::clone(self),
            },
            in_transaction: Cell::new(false),
        })
    }
}

enum Inner {
    Sqlite(rusqlite::Connection),
    Postgres {
        client: RefCell<Option<PgClient>>,
        pool: Arc<PgPool>,
    },
}

/// A connection to a database of the shared tier.
pub struct Connection {
    inner: Inner,
    in_transaction: Cell<bool>,
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let backend = match &self.inner {
            Inner::Sqlite(_) => "sqlite",
            Inner::Postgres { .. } => "postgres",
        };
        f.debug_struct("Connection")
            .field("backend", &backend)
            .finish_non_exhaustive()
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let Inner::Postgres { client, pool } = &self.inner else {
            return;
        };
        let Some(mut client) = client.borrow_mut().take() else {
            return;
        };
        // A connection that broke, or that a transaction left open, does
        // not go back to the pool.
        if self.in_transaction.get() && client.batch("rollback").is_err() {
            return;
        }
        let mut idle = pool.idle.lock().unwrap_or_else(PoisonError::into_inner);
        if idle.len() < PG_IDLE {
            idle.push(client);
        }
    }
}

impl From<rusqlite::Connection> for Connection {
    fn from(conn: rusqlite::Connection) -> Self {
        Self {
            inner: Inner::Sqlite(conn),
            in_transaction: Cell::new(false),
        }
    }
}

/// How a transaction starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionBehavior {
    /// The transaction takes its locks when it needs them.
    Deferred,
    /// The transaction is the one writer of the database from its start.
    Immediate,
}

impl Connection {
    /// Whether the connection is to Postgres.
    pub const fn is_postgres(&self) -> bool {
        matches!(self.inner, Inner::Postgres { .. })
    }

    fn run(&self, sql: &str, params: Params) -> Result<Outcome> {
        match &self.inner {
            Inner::Sqlite(conn) => run_sqlite(conn, sql, &params),
            Inner::Postgres { client, .. } => {
                let (sql, values) = for_postgres(sql, params);
                let mut client = client.borrow_mut();
                let Some(live) = client.as_mut() else {
                    return Err(PgError::Connection("the connection is closed".to_owned()).into());
                };
                match live.query(&sql, &values) {
                    Ok((changed, rows)) => {
                        Ok((usize::try_from(changed).unwrap_or(usize::MAX), rows))
                    }
                    Err(error) => {
                        // A connection that broke is not used again.
                        if matches!(error, PgError::Connection(_)) {
                            *client = None;
                        }
                        Err(error.into())
                    }
                }
            }
        }
    }

    /// Runs a statement and gives the number of rows it changed.
    pub fn execute(&self, sql: &str, params: impl IntoParams) -> Result<usize> {
        Ok(self.run(sql, params.into_params())?.0)
    }

    /// Runs statements without parameters, separated by semicolons. For
    /// Postgres a statement that creates a table or an index is given the
    /// types of Postgres.
    pub fn execute_batch(&self, sql: &str) -> Result<()> {
        match &self.inner {
            Inner::Sqlite(conn) => Ok(conn.execute_batch(sql)?),
            Inner::Postgres { client, .. } => {
                let mut client = client.borrow_mut();
                let Some(live) = client.as_mut() else {
                    return Err(PgError::Connection("the connection is closed".to_owned()).into());
                };
                Ok(live.batch(&ddl_for_postgres(sql))?)
            }
        }
    }

    /// Runs a query for one row.
    pub fn query_row<T>(
        &self,
        sql: &str,
        params: impl IntoParams,
        read: impl FnOnce(&Row<'_>) -> Result<T>,
    ) -> Result<T> {
        let (_, rows) = self.run(sql, params.into_params())?;
        let first = rows.first().ok_or(Error::NoRows)?;
        read(&Row { values: first })
    }

    /// A statement to run more than once, or to read rows from.
    pub fn prepare(&self, sql: &str) -> Result<Statement<'_>> {
        Ok(Statement {
            conn: self,
            sql: sql.to_owned(),
        })
    }

    /// [`Connection::prepare`]. SQLite keeps the compiled statement in
    /// both cases.
    pub fn prepare_cached(&self, sql: &str) -> Result<Statement<'_>> {
        self.prepare(sql)
    }

    /// Runs a query and gives its rows as values.
    pub fn query_values(&self, sql: &str, params: impl IntoParams) -> Result<Vec<Vec<Value>>> {
        Ok(self.run(sql, params.into_params())?.1)
    }

    /// The tables of the database, by name.
    pub fn tables(&self) -> Result<Vec<String>> {
        let sql = if self.is_postgres() {
            "select table_name from information_schema.tables where table_schema = current_schema() order by table_name"
        } else {
            "select name from sqlite_master where type = 'table' and name not like 'sqlite_%' order by name"
        };
        self.prepare(sql)?
            .query_map([], |row| row.get(0))?
            .collect()
    }

    /// The columns of a table, in the order of the table.
    pub fn columns(&self, table: &str) -> Result<Vec<String>> {
        if self.is_postgres() {
            return self
                .prepare("select column_name from information_schema.columns where table_schema = current_schema() and table_name = ?1 order by ordinal_position")?
                .query_map([table], |row| row.get(0))?
                .collect();
        }
        let quoted = table.replace('"', "");
        self.prepare(&format!(r#"pragma table_info("{quoted}")"#))?
            .query_map([], |row| row.get(1))?
            .collect()
    }

    /// Sets each sequence of a table of Postgres to the largest value of
    /// its column, so the next row gets the number after it. SQLite keeps
    /// the count itself.
    pub fn align_sequences(&self, table: &str) -> Result<()> {
        if !self.is_postgres() {
            return Ok(());
        }
        let serial: Vec<String> = self
            .prepare("select column_name from information_schema.columns where table_schema = current_schema() and table_name = ?1 and column_default like 'nextval(%'")?
            .query_map([table], |row| row.get(0))?
            .collect::<Result<_>>()?;
        let table = table.replace('"', "");
        for column in serial {
            let column = column.replace('"', "");
            self.query_values(
                &format!(
                    r#"select setval(pg_get_serial_sequence('"{table}"', '{column}'), coalesce(max("{column}"), 1), max("{column}") is not null) from "{table}""#
                ),
                [],
            )?;
        }
        Ok(())
    }

    /// Whether the database has the table.
    pub fn table_exists(&self, table: &str) -> Result<bool> {
        let sql = if self.is_postgres() {
            "select count(*) > 0 from information_schema.tables where table_schema = current_schema() and table_name = ?1"
        } else {
            "select count(*) > 0 from sqlite_master where type = 'table' and name = ?1"
        };
        self.query_row(sql, [table], |row| row.get(0))
    }

    /// Starts a transaction. It rolls back unless it is committed.
    pub fn transaction(&mut self) -> Result<Transaction<'_>> {
        self.transaction_with_behavior(TransactionBehavior::Deferred)
    }

    /// Starts a transaction of the kind `behavior`.
    pub fn transaction_with_behavior(
        &mut self,
        behavior: TransactionBehavior,
    ) -> Result<Transaction<'_>> {
        let immediate = behavior == TransactionBehavior::Immediate;
        match &self.inner {
            Inner::Sqlite(conn) => {
                conn.execute_batch(if immediate {
                    "begin immediate"
                } else {
                    "begin"
                })?;
            }
            Inner::Postgres { .. } => {
                self.run("begin", Params::Positional(Vec::new()))?;
            }
        }
        self.in_transaction.set(true);
        let transaction = Transaction { conn: self };
        if immediate && transaction.is_postgres() {
            // One writer for the schema until the transaction ends.
            transaction.run(
                "select pg_advisory_xact_lock(hashtext(current_schema()))",
                Params::Positional(Vec::new()),
            )?;
        }
        Ok(transaction)
    }

    fn end(&self, how: &str) -> Result<()> {
        self.in_transaction.set(false);
        match &self.inner {
            Inner::Sqlite(conn) => Ok(conn.execute_batch(how)?),
            Inner::Postgres { .. } => self.run(how, Params::Positional(Vec::new())).map(|_| ()),
        }
    }
}

/// A statement of a connection.
#[derive(Debug)]
pub struct Statement<'c> {
    conn: &'c Connection,
    sql: String,
}

impl Statement<'_> {
    /// Runs the statement and gives the number of rows it changed.
    pub fn execute(&mut self, params: impl IntoParams) -> Result<usize> {
        self.conn.execute(&self.sql, params)
    }

    /// Runs the query for one row.
    pub fn query_row<T>(
        &mut self,
        params: impl IntoParams,
        read: impl FnOnce(&Row<'_>) -> Result<T>,
    ) -> Result<T> {
        self.conn.query_row(&self.sql, params, read)
    }

    /// Runs the query and reads each row.
    pub fn query_map<T>(
        &mut self,
        params: impl IntoParams,
        mut read: impl FnMut(&Row<'_>) -> Result<T>,
    ) -> Result<std::vec::IntoIter<Result<T>>> {
        let (_, rows) = self.conn.run(&self.sql, params.into_params())?;
        let read: Vec<Result<T>> = rows.iter().map(|values| read(&Row { values })).collect();
        Ok(read.into_iter())
    }

    /// Whether the query finds a row.
    pub fn exists(&mut self, params: impl IntoParams) -> Result<bool> {
        Ok(!self.conn.run(&self.sql, params.into_params())?.1.is_empty())
    }
}

/// A transaction. It rolls back when it is dropped without a commit.
#[derive(Debug)]
pub struct Transaction<'c> {
    conn: &'c Connection,
}

impl std::ops::Deref for Transaction<'_> {
    type Target = Connection;

    fn deref(&self) -> &Connection {
        self.conn
    }
}

impl Transaction<'_> {
    /// Makes the changes of the transaction durable.
    pub fn commit(self) -> Result<()> {
        let done = self.conn.end("commit");
        std::mem::forget(self);
        done
    }
}

impl Drop for Transaction<'_> {
    fn drop(&mut self) {
        let _ = self.conn.end("rollback");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_statement_is_put_in_the_dialect_of_postgres() {
        let (sql, values) = for_postgres(
            r#"select "a:b" from "t" where "x" = ?1 and "y" = ?2 and 'it''s :no' <> ?1"#,
            params!["one", 2],
        );
        assert_eq!(
            sql,
            r#"select "a:b" from "t" where "x" = $1 and "y" = $2 and 'it''s :no' <> $1"#
        );
        assert_eq!(values, [Value::Text("one".to_owned()), Value::Integer(2)]);

        let (sql, values) = for_postgres(
            r#"insert into "t" ("did", "rev") values (:did, :rev) on conflict ("did") do update set "rev" = :rev"#,
            named_params! { ":rev": "r1", ":did": "d1" },
        );
        assert_eq!(
            sql,
            r#"insert into "t" ("did", "rev") values ($1, $2) on conflict ("did") do update set "rev" = $2"#
        );
        assert_eq!(
            values,
            [Value::Text("d1".to_owned()), Value::Text("r1".to_owned())]
        );
        // A cast keeps its two colons.
        assert_eq!(
            for_postgres("select 1::bigint", params![]).0,
            "select 1::bigint"
        );
    }

    #[test]
    fn a_table_of_the_reference_is_created_in_the_types_of_postgres() {
        let ddl = ddl_for_postgres(
            r#"CREATE TABLE "token" ("id" integer primary key autoincrement, "did" varchar not null, "integer text" blob, "n" integer default 0 not null)"#,
        );
        assert_eq!(
            ddl,
            r#"CREATE TABLE "token" ("id" bigserial primary key, "did" varchar collate "C" not null, "integer text" bytea, "n" bigint default 0 not null)"#
        );
    }

    const TABLE: &str = r#"create table "t" ("id" integer primary key autoincrement, "name" varchar not null unique, "data" blob, "flag" integer)"#;

    #[test]
    fn values_go_in_and_come_out_on_sqlite() {
        let conn = Connection::from(rusqlite::Connection::open_in_memory().unwrap());
        conn.execute_batch(TABLE).unwrap();
        exercise(conn);
    }

    /// The same statements on Postgres, when `PDS_TEST_POSTGRES_URL` names
    /// a server.
    #[test]
    fn values_go_in_and_come_out_on_postgres() {
        let Ok(url) = std::env::var("PDS_TEST_POSTGRES_URL") else {
            return;
        };
        let target = PgTarget::parse(&url).unwrap();
        let pool = PgPool::new(target, &format!("sql_test_{}", std::process::id()));
        pool.create_schema().unwrap();
        assert!(!pool.is_created().unwrap());
        let conn = pool.connect().unwrap();
        conn.execute_batch(TABLE).unwrap();
        assert!(pool.is_created().unwrap());
        exercise(conn);
        // The connection went back to the pool, and its transaction is over.
        let conn = pool.connect().unwrap();
        let unique = conn.execute(r#"insert into "t" ("name") values (?1)"#, ["b"]);
        assert!(matches!(unique, Err(Error::Postgres(error)) if error.is_unique_violation()));
        // After an error the connection takes the next statement.
        let count: i64 = conn
            .query_row(r#"select count(*) from "t""#, [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
        conn.execute_batch(&format!(r#"drop schema "{}" cascade"#, pool.schema()))
            .unwrap();
    }

    fn exercise(mut conn: Connection) {
        let id: i64 = conn
            .query_row(
                r#"insert into "t" ("name", "data", "flag") values (:name, :data, :flag) returning "id""#,
                named_params! { ":name": "a", ":data": vec![1_u8, 2], ":flag": true },
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(id, 1);
        assert_eq!(
            conn.execute(r#"insert into "t" ("name") values (?1)"#, ["b"])
                .unwrap(),
            1
        );
        let row: (String, Option<Vec<u8>>, Option<bool>) = conn
            .query_row(
                r#"select "name", "data", "flag" from "t" where "id" = ?1"#,
                [2],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(row, ("b".to_owned(), None, None));
        let names: Vec<String> = conn
            .prepare(r#"select "name" from "t" order by "id""#)
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_>>()
            .unwrap();
        assert_eq!(names, ["a", "b"]);
        let none = conn
            .query_row(r#"select "name" from "t" where "id" = 9"#, [], |row| {
                row.get::<String>(0)
            })
            .optional()
            .unwrap();
        assert_eq!(none, None);
        assert!(conn.table_exists("t").unwrap());
        assert!(!conn.table_exists("u").unwrap());
        // A wrong type is an error, not a default.
        assert!(matches!(
            conn.query_row(r#"select "name" from "t""#, [], |row| row.get::<i64>(0)),
            Err(Error::Type { index: 0, .. })
        ));

        // A transaction that is dropped changes nothing.
        {
            let txn = conn.transaction().unwrap();
            txn.execute(r#"delete from "t""#, []).unwrap();
        }
        let txn = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .unwrap();
        assert_eq!(
            txn.execute(r#"delete from "t" where "name" = ?1"#, ["a"])
                .unwrap(),
            1
        );
        txn.commit().unwrap();
        let count: i64 = conn
            .query_row(r#"select count(*) from "t""#, [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }
}
