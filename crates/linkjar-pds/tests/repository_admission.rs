//! Repository scheduling through the public worker and HTTP interfaces.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod common;

use common::{ALICE, DataDir};
use std::task::Poll;

async fn hold_writer(
    state: &pds_xrpc::AppState,
) -> (
    tokio::task::JoinHandle<Result<(), pds_xrpc::XrpcError>>,
    std::sync::mpsc::Sender<()>,
) {
    let (entered, started) = tokio::sync::oneshot::channel();
    let (release, held) = std::sync::mpsc::channel();
    let state = state.clone();
    let task = tokio::spawn(async move {
        state
            .repo_blocking(ALICE.to_owned(), move |state| {
                pds_xrpc::write::transact(state, ALICE, |_| {
                    let _ = entered.send(());
                    let _ = held.recv();
                    Ok(())
                })
            })
            .await
    });
    started.await.unwrap();
    (task, release)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_busy_repository_waits_without_taking_other_workers() {
    let dir = DataDir::new(1);
    let state = linkjar_pds::serve::app_state(&dir.config).unwrap();
    // Model ordinary work already occupying most of the shared budget.
    let _ordinary: Vec<_> = (0..62)
        .map(|_| state.resources().blocking().unwrap())
        .collect();
    let (first, release) = hold_writer(&state).await;
    let mut waiting = Vec::new();
    for _ in 0..7 {
        let mut work = Box::pin(state.repo_blocking(ALICE.to_owned(), |state| {
            pds_xrpc::write::transact(state, ALICE, |_| Ok(()))
        }));
        assert!(
            matches!(futures_util::poll!(work.as_mut()), Poll::Pending),
            "repository work must wait without consuming another ordinary worker"
        );
        waiting.push(work);
    }
    let refusal = state
        .repo_blocking(ALICE.to_owned(), |_| Ok(()))
        .await
        .unwrap_err();
    assert_eq!(
        refusal.status, 503,
        "a repository queue must remain bounded"
    );
    assert_eq!(
        state.blocking(|_| Ok("available")).await.unwrap(),
        "available"
    );
    release.send(()).unwrap();
    first.await.unwrap().unwrap();
    for work in waiting {
        work.await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn cancellation_releases_waiters_but_keeps_the_running_writer_admitted() {
    let dir = DataDir::new(1);
    let state = linkjar_pds::serve::app_state(&dir.config).unwrap();
    let (first, release) = hold_writer(&state).await;
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());

    let mut waiting = Vec::new();
    let (order, mut finished) = tokio::sync::mpsc::unbounded_channel();
    for index in 0..7 {
        let order = order.clone();
        let mut work = Box::pin(state.repo_blocking(ALICE.to_owned(), move |_| {
            order.send(index).unwrap();
            Ok(())
        }));
        assert!(matches!(futures_util::poll!(work.as_mut()), Poll::Pending));
        waiting.push(work);
    }
    assert_eq!(
        state
            .repo_blocking(ALICE.to_owned(), |_| Ok(()))
            .await
            .unwrap_err()
            .status,
        503,
        "a disconnected writer retains its place until its worker exits"
    );
    // Cancelling a waiter frees a place immediately, without starting its work.
    drop(waiting.remove(3));
    let mut replacement = Box::pin(state.repo_blocking(ALICE.to_owned(), move |_| {
        order.send(7).unwrap();
        Ok(())
    }));
    assert!(matches!(
        futures_util::poll!(replacement.as_mut()),
        Poll::Pending
    ));
    release.send(()).unwrap();
    for result in futures_util::future::join_all(waiting).await {
        result.unwrap();
    }
    replacement.await.unwrap();
    let mut actual = Vec::new();
    while let Ok(index) = finished.try_recv() {
        actual.push(index);
    }
    assert_eq!(actual, [0, 1, 2, 4, 5, 6, 7]);
    state
        .repo_blocking(ALICE.to_owned(), |_| Ok(()))
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn repository_admission_has_a_global_bound_and_recovers_cancelled_capacity() {
    let dir = DataDir::new(1);
    let state = linkjar_pds::serve::app_state(&dir.config).unwrap();
    let mut admitted = Vec::new();
    let mut releases = Vec::new();
    for index in 0..64 {
        let (release, held) = std::sync::mpsc::channel();
        let mut work = Box::pin(state.repo_blocking(
            format!("did:plc:group{}", index / 8),
            move |_| {
                let _ = held.recv();
                Ok(())
            },
        ));
        assert!(matches!(futures_util::poll!(work.as_mut()), Poll::Pending));
        admitted.push(work);
        releases.push(release);
    }
    assert_eq!(
        state
            .repo_blocking(ALICE.to_owned(), |_| Ok(()))
            .await
            .unwrap_err()
            .status,
        503
    );
    // The last job waits behind its group's writer; cancellation is immediate.
    drop(admitted.pop());
    state
        .repo_blocking(ALICE.to_owned(), |_| Ok(()))
        .await
        .unwrap();
    for release in releases {
        let _ = release.send(());
    }
    for result in futures_util::future::join_all(admitted).await {
        result.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn http_writes_observe_repository_admission_while_reads_remain_available() {
    let dir = DataDir::new(1);
    let state = linkjar_pds::serve::app_state(&dir.config).unwrap();
    let listener = common::http::start(pds_xrpc::router(state.clone())).await;
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()
        .unwrap();
    let login = common::http::post(
        &http,
        &listener.origin,
        "com.atproto.server.createSession",
        None,
        Some(&serde_json::json!({"identifier": "alice.pds.test", "password": common::PASSWORD})),
    )
    .send()
    .await
    .unwrap();
    let (status, login) = common::http::json_response(login).await;
    assert_eq!(status, 200);
    let (first, release) = hold_writer(&state).await;
    let mut waiting = Vec::new();
    for _ in 0..7 {
        let mut work = Box::pin(state.repo_blocking(ALICE.to_owned(), |_| Ok(())));
        assert!(matches!(futures_util::poll!(work.as_mut()), Poll::Pending));
        waiting.push(work);
    }
    let request = serde_json::json!({"repo": ALICE, "collection": common::POST, "rkey": "later", "record": {"$type": common::POST, "text": "later", "createdAt": "2026-01-01T00:00:00.000Z"}});
    let response = common::http::post(
        &http,
        &listener.origin,
        "com.atproto.repo.createRecord",
        login["accessJwt"].as_str(),
        Some(&request),
    )
    .send()
    .await
    .unwrap();
    let (status, refusal) = common::http::json_response(response).await;
    assert_eq!(status, 503);
    assert_eq!(refusal["error"], "ServerBusy");
    let read = http
        .get(format!(
            "{}/xrpc/com.atproto.repo.getRecord?repo={ALICE}&collection={}&rkey=r0000",
            listener.origin,
            common::POST
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(read.status(), 200);
    release.send(()).unwrap();
    first.await.unwrap().unwrap();
    for result in futures_util::future::join_all(waiting).await {
        result.unwrap();
    }
}
