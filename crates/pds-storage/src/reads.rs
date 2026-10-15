//! Read connections kept between requests (`docs/SPEC.md` §8.3).
//!
//! A fresh connection to a store of 20,000 records costs 150 to 250 µs
//! with its first statement: the open, the schema, the compiled statement.
//! The same statement on a kept connection costs 2 to 5 µs
//! (`tests/pool.rs`, the ignored measurement). A `getRecord` opened two:
//! the accounts and the store. The pool keeps the read connections of the
//! last requests: a free list for `account.sqlite`, and an LRU by DID for
//! the actor stores, as the write path keeps its write connections.
//!
//! A connection is lent out by value, because SQLite connections are
//! `Send` and not `Sync`, and goes back when the loan is dropped: at the
//! end of the request, or at the end of the body that streams from it.
//!
//! **The WAL.** A kept connection runs each statement in a read
//! transaction of its own and holds none between statements, so it pins no
//! WAL snapshot while it waits: the writer's checkpoints go on, and the
//! `-wal` file is reset as it is without the pool. What stays is the
//! `-wal` and `-shm` files themselves: SQLite removes them when the last
//! connection closes, and with the pool a connection stays open.
//! `tests/pool.rs` shows the checkpoint.
//!
//! **A store that goes away.** When an account is deleted its directory is
//! removed. [`ReadPool::forget_store`] closes the kept connections of the
//! DID, and a connection lent out before that is closed when it comes
//! back, not kept: it holds the old file open, and a store of the same DID
//! that comes later is another file.
//!
//! A snapshot ([`crate::ActorDb::open_snapshot`]) holds a read transaction
//! and is never kept. The shared tier on Postgres has a pool of its own
//! ([`crate::sql::PgPool`]): there the loans of `account.sqlite` wrap its
//! connections and keep nothing here.

use std::ops::Deref;
use std::sync::{Arc, Mutex, PoisonError};

use crate::account::AccountDb;
use crate::actor::ActorDb;
use crate::error::StorageError;
use crate::layout::DataLayout;

/// How many read connections to `account.sqlite` wait for a request. More
/// requests at once open connections of their own, which close when they
/// come back to a full list.
const ACCOUNT_IDLE: usize = 8;

/// The read connections of the actor stores, the one used last at the
/// back, with the count of the stores forgotten so far.
struct Stores {
    kept: Vec<(String, ActorDb)>,
    /// Grows by one each time a store is forgotten. A loan taken before
    /// the growth is closed when it comes back.
    epoch: u64,
}

/// The kept read connections of one data directory.
pub struct ReadPool {
    layout: DataLayout,
    accounts: Mutex<Vec<AccountDb>>,
    stores: Mutex<Stores>,
    /// How many actor stores stay open for the next read.
    store_capacity: usize,
}

impl std::fmt::Debug for ReadPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (accounts, stores) = self.kept();
        f.debug_struct("ReadPool")
            .field("store_capacity", &self.store_capacity)
            .field("accounts", &accounts)
            .field("stores", &stores)
            .finish_non_exhaustive()
    }
}

impl ReadPool {
    /// A pool over `layout` that keeps up to `store_capacity` read
    /// connections to actor stores.
    pub fn new(layout: DataLayout, store_capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            layout,
            accounts: Mutex::new(Vec::new()),
            stores: Mutex::new(Stores {
                kept: Vec::new(),
                epoch: 0,
            }),
            store_capacity,
        })
    }

    /// The data directory.
    pub const fn layout(&self) -> &DataLayout {
        &self.layout
    }

    /// How many connections wait in the pool: to `account.sqlite`, and to
    /// actor stores.
    pub fn kept(&self) -> (usize, usize) {
        let accounts = self
            .accounts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len();
        let stores = self
            .stores
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .kept
            .len();
        (accounts, stores)
    }

    /// A read connection to the accounts: a kept one, or a new one.
    pub fn accounts(self: &Arc<Self>) -> Result<AccountsLoan, StorageError> {
        let kept = self
            .accounts
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop();
        let db = match kept {
            Some(db) => db,
            None => self.layout.accounts()?,
        };
        Ok(AccountsLoan {
            db: Some(db),
            pool: Arc::clone(self),
        })
    }

    fn keep_accounts(&self, db: AccountDb) {
        // The connections of the shared tier go back to its own pool.
        if self.layout.shared.is_some() {
            return;
        }
        let mut idle = self.accounts.lock().unwrap_or_else(PoisonError::into_inner);
        if idle.len() < ACCOUNT_IDLE {
            idle.push(db);
        }
    }

    /// A read connection to the actor store of `did`: the one kept from
    /// the last read, or a new one. A store that is not there is an
    /// error, as for [`ActorDb::open`].
    pub fn store(self: &Arc<Self>, did: &str) -> Result<StoreLoan, StorageError> {
        let (kept, epoch) = {
            let mut stores = self.stores.lock().unwrap_or_else(PoisonError::into_inner);
            let kept = stores
                .kept
                .iter()
                .position(|(owner, _)| owner == did)
                .map(|index| stores.kept.remove(index).1);
            (kept, stores.epoch)
        };
        let db = match kept {
            Some(db) => db,
            None => ActorDb::open(&self.layout.actor_db(did))?,
        };
        Ok(StoreLoan {
            db: Some(db),
            did: did.to_owned(),
            epoch,
            pool: Arc::clone(self),
        })
    }

    fn keep_store(&self, did: String, db: ActorDb, epoch: u64) {
        if self.store_capacity == 0 {
            return;
        }
        let mut stores = self.stores.lock().unwrap_or_else(PoisonError::into_inner);
        if stores.epoch != epoch {
            // A store was forgotten while this connection was out: it may
            // hold the removed file.
            return;
        }
        stores.kept.push((did, db));
        let closed = (stores.kept.len() > self.store_capacity).then(|| stores.kept.remove(0));
        drop(stores);
        // The connection closes outside the lock.
        drop(closed);
    }

    /// Closes the connections kept for the actor store of `did`, and
    /// keeps none that is out now: the store is about to be removed.
    pub fn forget_store(&self, did: &str) {
        let closed = {
            let mut stores = self.stores.lock().unwrap_or_else(PoisonError::into_inner);
            let (gone, kept): (Vec<_>, Vec<_>) = std::mem::take(&mut stores.kept)
                .into_iter()
                .partition(|(owner, _)| owner == did);
            stores.kept = kept;
            stores.epoch += 1;
            gone
        };
        drop(closed);
    }
}

/// A read connection to the accounts on loan from a [`ReadPool`]. It goes
/// back when the value is dropped.
pub struct AccountsLoan {
    db: Option<AccountDb>,
    pool: Arc<ReadPool>,
}

impl std::fmt::Debug for AccountsLoan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountsLoan").finish_non_exhaustive()
    }
}

impl Deref for AccountsLoan {
    type Target = AccountDb;

    #[allow(clippy::expect_used)] // the option is `None` only inside `drop`
    fn deref(&self) -> &AccountDb {
        self.db.as_ref().expect("the loan was returned")
    }
}

impl Drop for AccountsLoan {
    fn drop(&mut self) {
        if let Some(db) = self.db.take() {
            self.pool.keep_accounts(db);
        }
    }
}

/// A read connection to one actor store on loan from a [`ReadPool`]. It
/// goes back when the value is dropped.
pub struct StoreLoan {
    db: Option<ActorDb>,
    did: String,
    epoch: u64,
    pool: Arc<ReadPool>,
}

impl std::fmt::Debug for StoreLoan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StoreLoan")
            .field("did", &self.did)
            .finish_non_exhaustive()
    }
}

impl Deref for StoreLoan {
    type Target = ActorDb;

    #[allow(clippy::expect_used)] // the option is `None` only inside `drop`
    fn deref(&self) -> &ActorDb {
        self.db.as_ref().expect("the loan was returned")
    }
}

impl Drop for StoreLoan {
    fn drop(&mut self) {
        if let Some(db) = self.db.take() {
            self.pool
                .keep_store(std::mem::take(&mut self.did), db, self.epoch);
        }
    }
}
