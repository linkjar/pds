//! The dispatcher of sign-in notices (`docs/SPEC.md` §12.6; patch 100).
//!
//! A change of a sign-in method writes a notice in the transaction of the
//! change. This dispatcher sends the notices that are due. A notice that
//! is not delivered stays, and waits longer after each failure: from one
//! minute to one hour. Delivery is at least once.

use std::time::Duration;

use super::{accounts, iso};
use crate::state::AppState;

/// How many notices one pass sends.
const BATCH: u32 = 100;
/// How often the dispatcher looks for notices.
pub const INTERVAL: Duration = Duration::from_secs(60);
const FIRST_RETRY_MS: i64 = 60 * 1000;
const LAST_RETRY_MS: i64 = 60 * 60 * 1000;

/// How long a notice waits after its `failures`th failed delivery.
fn retry_delay_ms(failures: i64) -> i64 {
    let doublings = u32::try_from(failures.clamp(0, 16)).unwrap_or(16);
    FIRST_RETRY_MS
        .saturating_mul(1_i64 << doublings)
        .min(LAST_RETRY_MS)
}

/// Sends the notices that are due. Returns how many were delivered.
pub async fn dispatch_once(state: &AppState) -> usize {
    let now_ms = state.now_ms();
    let Ok(due) = accounts(state, move |db| db.due_notices(now_ms, BATCH)).await else {
        return 0;
    };
    let providers = state.config().profile.providers();
    let mut delivered = 0;
    for notice in due {
        let method = notice
            .provider
            .as_deref()
            .and_then(|id| providers.iter().find(|provider| provider.id == id))
            .map_or("A sign-in method", |provider| provider.label);
        let change = match notice.action.as_str() {
            "password-set" => "Your account password was set or changed.".to_owned(),
            "linked" => format!("{method} was linked to your account."),
            _ => format!("{method} was unlinked from your account."),
        };
        let sent = state
            .config()
            .mail
            .send_notice(
                &notice.id,
                &notice.recipient,
                &change,
                &notice.did,
                &iso(notice.created_ms),
            )
            .await;
        let id = notice.id.clone();
        let stored = if sent.is_ok() {
            delivered += 1;
            accounts(state, move |db| db.delete_notice(&id)).await
        } else {
            // The cause may name an address, so it stays out of the log.
            let next = now_ms + retry_delay_ms(notice.attempts);
            accounts(state, move |db| db.retry_notice(&id, next)).await
        };
        if stored.is_err() {
            tracing::warn!("a sign-in notice could not be updated");
        }
    }
    delivered
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notice_waits_from_one_minute_to_one_hour() {
        assert_eq!(retry_delay_ms(0), 60_000);
        assert_eq!(retry_delay_ms(1), 120_000);
        assert_eq!(retry_delay_ms(5), 1_920_000);
        assert_eq!(retry_delay_ms(6), 3_600_000);
        assert_eq!(retry_delay_ms(1000), 3_600_000);
        assert_eq!(retry_delay_ms(-1), 60_000);
    }
}
