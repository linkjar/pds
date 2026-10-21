//! The OAuth rows of `account.sqlite` (`docs/SPEC.md` §8.1, §11.3).
//!
//! They are pushed requests, devices, the accounts a device remembers, the
//! clients an account authorized, and tokens. The tables and the values are
//! the Reference's, so a session opened before the cutover goes on after it.
//!
//! Times are ISO 8601 text, and `parameters` and `clientAuth` are JSON
//! text, as the Reference writes them.

use crate::sql::{OptionalExtension, Row, params};

use crate::account::AccountDb;
use crate::error::StorageError;

/// A pushed authorization request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestRow {
    /// `req-` and 32 hex digits.
    pub id: String,
    /// The account that authorized it.
    pub did: Option<String>,
    /// The device that opened it.
    pub device_id: Option<String>,
    /// The client.
    pub client_id: String,
    /// How the client authenticated, as JSON.
    pub client_auth: String,
    /// The request parameters, as JSON.
    pub parameters: String,
    /// When it ends.
    pub expires_at: String,
    /// The code, once authorized.
    pub code: Option<String>,
}

const REQUEST_COLUMNS: &str =
    r#""id", "did", "deviceId", "clientId", "clientAuth", "parameters", "expiresAt", "code""#;

fn request_from_row(row: &Row<'_>) -> crate::sql::Result<RequestRow> {
    Ok(RequestRow {
        id: row.get(0)?,
        did: row.get(1)?,
        device_id: row.get(2)?,
        client_id: row.get(3)?,
        client_auth: row.get(4)?,
        parameters: row.get(5)?,
        expires_at: row.get(6)?,
        code: row.get(7)?,
    })
}

/// A browser the server knows by its cookies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceRow {
    /// `dev-` and 32 hex digits.
    pub id: String,
    /// `ses-` and 32 hex digits. It rotates.
    pub session_id: String,
    /// The `User-Agent` last seen.
    pub user_agent: Option<String>,
    /// The address last seen.
    pub ip_address: String,
    /// When the server last saw it.
    pub last_seen_at: String,
}

/// An account a device remembers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceAccount {
    /// The account.
    pub did: String,
    /// When the account last signed in on the device.
    pub updated_at: String,
}

/// A session of an OAuth client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenRow {
    /// The row number, which a rotation keeps.
    pub id: i64,
    /// The account.
    pub did: String,
    /// `tok-` and 32 hex digits: the `jti` of the access token. It rotates.
    pub token_id: String,
    /// When the session began.
    pub created_at: String,
    /// When it was last refreshed.
    pub updated_at: String,
    /// When the access token ends.
    pub expires_at: String,
    /// The client.
    pub client_id: String,
    /// How the client authenticated, as JSON.
    pub client_auth: String,
    /// The device the session began on.
    pub device_id: Option<String>,
    /// The request parameters, as JSON.
    pub parameters: String,
    /// The code the session came from.
    pub code: Option<String>,
    /// The refresh token that is good now.
    pub current_refresh_token: Option<String>,
    /// The scope granted. An old row has none and the parameters say it.
    pub scope: Option<String>,
}

const TOKEN_COLUMNS: &str = r#""id", "did", "tokenId", "createdAt", "updatedAt", "expiresAt", "clientId", "clientAuth", "deviceId", "parameters", "code", "currentRefreshToken", "scope""#;

fn token_from_row(row: &Row<'_>) -> crate::sql::Result<TokenRow> {
    Ok(TokenRow {
        id: row.get(0)?,
        did: row.get(1)?,
        token_id: row.get(2)?,
        created_at: row.get(3)?,
        updated_at: row.get(4)?,
        expires_at: row.get(5)?,
        client_id: row.get(6)?,
        client_auth: row.get(7)?,
        device_id: row.get(8)?,
        parameters: row.get(9)?,
        code: row.get(10)?,
        current_refresh_token: row.get(11)?,
        scope: row.get(12)?,
    })
}

/// What a rotation writes.
#[derive(Debug, Clone, Copy)]
pub struct Rotation<'a> {
    /// The new `jti`.
    pub token_id: &'a str,
    /// The new refresh token.
    pub refresh_token: &'a str,
    /// The time of the rotation.
    pub updated_at: &'a str,
    /// When the new access token ends.
    pub expires_at: &'a str,
    /// How the client authenticated this time, as JSON.
    pub client_auth: &'a str,
    /// The scope.
    pub scope: &'a str,
}

impl AccountDb {
    /// Stores a pushed request.
    pub fn create_request(&self, request: &RequestRow) -> Result<(), StorageError> {
        self.conn.execute(
            r#"insert into "authorization_request" ("id", "did", "deviceId", "clientId", "clientAuth", "parameters", "expiresAt", "code") values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)"#,
            params![
                request.id,
                request.did,
                request.device_id,
                request.client_id,
                request.client_auth,
                request.parameters,
                request.expires_at,
                request.code
            ],
        )?;
        Ok(())
    }

    /// Reads a pushed request.
    pub fn request(&self, id: &str) -> Result<Option<RequestRow>, StorageError> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    r#"select {REQUEST_COLUMNS} from "authorization_request" where "id" = ?1"#
                ),
                [id],
                request_from_row,
            )
            .optional()?)
    }

    /// Writes a request back after a step of the flow.
    pub fn update_request(&self, request: &RequestRow) -> Result<(), StorageError> {
        self.conn.execute(
            r#"update "authorization_request" set "did" = ?2, "deviceId" = ?3, "parameters" = ?4, "expiresAt" = ?5, "code" = ?6 where "id" = ?1"#,
            params![
                request.id,
                request.did,
                request.device_id,
                request.parameters,
                request.expires_at,
                request.code
            ],
        )?;
        Ok(())
    }

    /// Associates a sign-in awaiting consent with its account so recovery
    /// can revoke it even when the browser does not remember the account.
    pub fn authenticate_request(&self, id: &str, did: &str) -> Result<bool, StorageError> {
        Ok(self.conn.execute(
            r#"update "authorization_request" set "did" = ?2 where "id" = ?1 and "code" is null"#,
            [id, did],
        )? > 0)
    }

    /// Removes a request.
    pub fn delete_request(&self, id: &str) -> Result<(), StorageError> {
        self.conn.execute(
            r#"delete from "authorization_request" where "id" = ?1"#,
            [id],
        )?;
        Ok(())
    }

    /// Removes the requests that ended before `now`.
    pub fn delete_expired_requests(&self, now: &str) -> Result<(), StorageError> {
        self.conn.execute(
            r#"delete from "authorization_request" where "expiresAt" < ?1"#,
            [now],
        )?;
        Ok(())
    }

    /// Finds the account of a code before taking its account lock. Callers
    /// must re-read or consume the request while holding that lock.
    pub fn request_by_code(&self, code: &str) -> Result<Option<RequestRow>, StorageError> {
        Ok(self
            .conn
            .query_row(
                &format!(
                    r#"select {REQUEST_COLUMNS} from "authorization_request" where "code" = ?1"#
                ),
                [code],
                request_from_row,
            )
            .optional()?)
    }

    /// Takes the request of a code. A code is good once: the row goes with
    /// the read.
    pub fn consume_request_code(&mut self, code: &str) -> Result<Option<RequestRow>, StorageError> {
        let txn = self.conn.transaction()?;
        let found = txn
            .query_row(
                &format!(
                    r#"select {REQUEST_COLUMNS} from "authorization_request" where "code" = ?1"#
                ),
                [code],
                request_from_row,
            )
            .optional()?;
        if let Some(request) = &found {
            txn.execute(
                r#"delete from "authorization_request" where "id" = ?1"#,
                [&request.id],
            )?;
        }
        txn.commit()?;
        Ok(found)
    }

    /// Stores a new device.
    pub fn create_device(&self, device: &DeviceRow) -> Result<(), StorageError> {
        self.conn.execute(
            r#"insert into "device" ("id", "sessionId", "userAgent", "ipAddress", "lastSeenAt") values (?1, ?2, ?3, ?4, ?5)"#,
            params![
                device.id,
                device.session_id,
                device.user_agent,
                device.ip_address,
                device.last_seen_at
            ],
        )?;
        Ok(())
    }

    /// Reads a device.
    pub fn device(&self, id: &str) -> Result<Option<DeviceRow>, StorageError> {
        Ok(self
            .conn
            .query_row(
                r#"select "id", "sessionId", "userAgent", "ipAddress", "lastSeenAt" from "device" where "id" = ?1"#,
                [id],
                |row| {
                    Ok(DeviceRow {
                        id: row.get(0)?,
                        session_id: row.get(1)?,
                        user_agent: row.get(2)?,
                        ip_address: row.get(3)?,
                        last_seen_at: row.get(4)?,
                    })
                },
            )
            .optional()?)
    }

    /// Writes a device back.
    pub fn update_device(&self, device: &DeviceRow) -> Result<(), StorageError> {
        self.conn.execute(
            r#"update "device" set "sessionId" = ?2, "userAgent" = ?3, "ipAddress" = ?4, "lastSeenAt" = ?5 where "id" = ?1"#,
            params![
                device.id,
                device.session_id,
                device.user_agent,
                device.ip_address,
                device.last_seen_at
            ],
        )?;
        Ok(())
    }

    /// Removes a device and what it remembers.
    pub fn delete_device(&mut self, id: &str) -> Result<(), StorageError> {
        let txn = self.conn.transaction()?;
        txn.execute(
            r#"delete from "account_device" where "deviceId" = ?1"#,
            [id],
        )?;
        txn.execute(r#"delete from "device" where "id" = ?1"#, [id])?;
        txn.commit()?;
        Ok(())
    }

    /// Makes the device remember the account, or renews the memory.
    pub fn upsert_device_account(
        &self,
        device_id: &str,
        did: &str,
        now: &str,
    ) -> Result<(), StorageError> {
        self.conn.execute(
            r#"insert into "account_device" ("did", "deviceId", "createdAt", "updatedAt") values (?1, ?2, ?3, ?3) on conflict ("deviceId", "did") do update set "updatedAt" = ?3"#,
            params![did, device_id, now],
        )?;
        Ok(())
    }

    /// Makes the device forget the account.
    pub fn remove_device_account(&self, device_id: &str, did: &str) -> Result<(), StorageError> {
        self.conn.execute(
            r#"delete from "account_device" where "deviceId" = ?1 and "did" = ?2"#,
            params![device_id, did],
        )?;
        Ok(())
    }

    /// The accounts the device remembers, the latest sign-in first.
    pub fn device_accounts(&self, device_id: &str) -> Result<Vec<DeviceAccount>, StorageError> {
        let mut statement = self.conn.prepare(
            r#"select account_device."did", account_device."updatedAt" from "account_device" inner join "actor" on actor."did" = account_device."did" where account_device."deviceId" = ?1 and actor."takedownRef" is null order by account_device."updatedAt" desc"#,
        )?;
        let rows = statement.query_map([device_id], |row| {
            Ok(DeviceAccount {
                did: row.get(0)?,
                updated_at: row.get(1)?,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Records what the account granted the client, as JSON.
    pub fn set_authorized_client(
        &self,
        did: &str,
        client_id: &str,
        data: &str,
        now: &str,
    ) -> Result<(), StorageError> {
        self.conn.execute(
            r#"insert into "authorized_client" ("did", "clientId", "createdAt", "updatedAt", "data") values (?1, ?2, ?3, ?3, ?4) on conflict ("did", "clientId") do update set "updatedAt" = ?3, "data" = ?4"#,
            params![did, client_id, now, data],
        )?;
        Ok(())
    }

    /// What the account granted the client, as JSON.
    pub fn authorized_client(
        &self,
        did: &str,
        client_id: &str,
    ) -> Result<Option<String>, StorageError> {
        Ok(self
            .conn
            .query_row(
                r#"select "data" from "authorized_client" where "did" = ?1 and "clientId" = ?2"#,
                [did, client_id],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Stores a new session of a client.
    pub fn create_token(&self, token: &TokenRow) -> Result<(), StorageError> {
        self.conn.execute(
            r#"insert into "token" ("did", "tokenId", "createdAt", "updatedAt", "expiresAt", "clientId", "clientAuth", "deviceId", "parameters", "details", "code", "currentRefreshToken", "scope") values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, null, ?10, ?11, ?12)"#,
            params![
                token.did,
                token.token_id,
                token.created_at,
                token.updated_at,
                token.expires_at,
                token.client_id,
                token.client_auth,
                token.device_id,
                token.parameters,
                token.code,
                token.current_refresh_token,
                token.scope
            ],
        )?;
        Ok(())
    }

    fn token_where(&self, clause: &str, value: &str) -> Result<Option<TokenRow>, StorageError> {
        Ok(self
            .conn
            .query_row(
                &format!(r#"select {TOKEN_COLUMNS} from "token" where {clause} = ?1"#),
                [value],
                token_from_row,
            )
            .optional()?)
    }

    /// The session whose access token has this `jti`.
    pub fn token(&self, token_id: &str) -> Result<Option<TokenRow>, StorageError> {
        self.token_where(r#""tokenId""#, token_id)
    }

    /// The session that came from a code.
    pub fn token_by_code(&self, code: &str) -> Result<Option<TokenRow>, StorageError> {
        self.token_where(r#""code""#, code)
    }

    /// The session a refresh token belongs to: the one it is good for now,
    /// or the one that used it up.
    pub fn token_by_refresh_token(
        &self,
        refresh_token: &str,
    ) -> Result<Option<TokenRow>, StorageError> {
        let used: Option<i64> = self
            .conn
            .query_row(
                r#"select "tokenId" from "used_refresh_token" where "refreshToken" = ?1"#,
                [refresh_token],
                |row| row.get(0),
            )
            .optional()?;
        match used {
            Some(id) => Ok(self
                .conn
                .query_row(
                    &format!(r#"select {TOKEN_COLUMNS} from "token" where "id" = ?1"#),
                    [id],
                    token_from_row,
                )
                .optional()?),
            None => self.token_where(r#""currentRefreshToken""#, refresh_token),
        }
    }

    /// The sessions of an account, the latest refresh first.
    pub fn account_tokens(&self, did: &str) -> Result<Vec<TokenRow>, StorageError> {
        let mut statement = self.conn.prepare(&format!(
            r#"select {TOKEN_COLUMNS} from "token" where "did" = ?1 order by "updatedAt" desc"#
        ))?;
        let rows = statement.query_map([did], token_from_row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// The devices that remember an account, each with the time of the
    /// account's last sign-in on it, the latest first.
    pub fn account_devices(&self, did: &str) -> Result<Vec<(DeviceRow, String)>, StorageError> {
        let mut statement = self.conn.prepare(
            r#"select device."id", device."sessionId", device."userAgent", device."ipAddress", device."lastSeenAt", account_device."updatedAt" from "account_device" inner join "device" on device."id" = account_device."deviceId" where account_device."did" = ?1 order by account_device."updatedAt" desc"#,
        )?;
        let rows = statement.query_map([did], |row| {
            Ok((
                DeviceRow {
                    id: row.get(0)?,
                    session_id: row.get(1)?,
                    user_agent: row.get(2)?,
                    ip_address: row.get(3)?,
                    last_seen_at: row.get(4)?,
                },
                row.get(5)?,
            ))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Ends a session.
    pub fn delete_token(&mut self, token_id: &str) -> Result<(), StorageError> {
        let txn = self.conn.transaction()?;
        txn.execute(
            r#"delete from "used_refresh_token" where "tokenId" in (select "id" from "token" where "tokenId" = ?1)"#,
            [token_id],
        )?;
        txn.execute(r#"delete from "token" where "tokenId" = ?1"#, [token_id])?;
        txn.commit()?;
        Ok(())
    }

    /// Ends every session of an account.
    pub fn delete_account_tokens(&mut self, did: &str) -> Result<(), StorageError> {
        let txn = self.conn.transaction()?;
        txn.execute(
            r#"delete from "used_refresh_token" where "tokenId" in (select "id" from "token" where "did" = ?1)"#,
            [did],
        )?;
        txn.execute(r#"delete from "token" where "did" = ?1"#, [did])?;
        txn.commit()?;
        Ok(())
    }

    /// Rotates the session `id`: the refresh token in use becomes a used
    /// one, and the row takes the new `jti` and refresh token. `false`
    /// means the new refresh token was used before, and nothing but the
    /// record of the old one changed.
    pub fn rotate_token(&mut self, id: i64, next: Rotation<'_>) -> Result<bool, StorageError> {
        let txn = self.conn.transaction()?;
        let current: Option<String> = txn.query_row(
            r#"select "currentRefreshToken" from "token" where "id" = ?1"#,
            [id],
            |row| row.get(0),
        )?;
        if let Some(current) = current {
            txn.execute(
                r#"insert into "used_refresh_token" ("tokenId", "refreshToken") values (?1, ?2) on conflict do nothing"#,
                params![id, current],
            )?;
        }
        let reused: i64 = txn.query_row(
            r#"select count(*) from "used_refresh_token" where "refreshToken" = ?1"#,
            [next.refresh_token],
            |row| row.get(0),
        )?;
        if reused == 0 {
            txn.execute(
                r#"update "token" set "tokenId" = ?2, "currentRefreshToken" = ?3, "expiresAt" = ?4, "updatedAt" = ?5, "clientAuth" = ?6, "scope" = ?7 where "id" = ?1"#,
                params![
                    id,
                    next.token_id,
                    next.refresh_token,
                    next.expires_at,
                    next.updated_at,
                    next.client_auth,
                    next.scope
                ],
            )?;
        }
        txn.commit()?;
        Ok(reused == 0)
    }
}
