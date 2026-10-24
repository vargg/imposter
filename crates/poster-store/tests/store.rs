//! The database actor: one transaction per command, in order, on its own thread.
#![cfg(test)]

mod support;

use poster_core::model::Variables;
use poster_store::collections::{collections, create_collection};
use poster_store::{Store, StoreError};
use support::store;

#[test]
fn a_command_commits_when_it_succeeds() {
    let (_dir, store) = store();

    store
        .call_blocking(|db| create_collection(db, "API", "", &Variables::new()))
        .unwrap();

    let names: Vec<String> = store
        .call_blocking(|db| collections(db))
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(names, ["API"]);
}

#[test]
fn a_failing_command_rolls_back_every_write_it_made() {
    let (_dir, store) = store();

    let result: Result<(), StoreError> = store.call_blocking(|db| {
        create_collection(db, "first", "", &Variables::new())?;
        create_collection(db, "second", "", &Variables::new())?;
        Err(StoreError::NotFound("thing"))
    });

    assert!(matches!(result, Err(StoreError::NotFound("thing"))));
    assert_eq!(store.call_blocking(|db| collections(db)).unwrap(), []);
}

#[test]
fn a_panicking_command_rolls_back_and_the_database_keeps_working() {
    let (_dir, store) = store();

    let result: Result<(), StoreError> = store.call_blocking(|db| {
        create_collection(db, "lost", "", &Variables::new())?;
        panic!("bug inside a command");
    });

    assert!(matches!(result, Err(StoreError::Panicked)));
    assert_eq!(store.call_blocking(|db| collections(db)).unwrap(), []);
    store
        .call_blocking(|db| create_collection(db, "kept", "", &Variables::new()))
        .unwrap();
}

#[test]
fn commands_from_many_threads_run_one_at_a_time_in_send_order_per_thread() {
    let (_dir, store) = store();
    let handles: Vec<_> = (0..4)
        .map(|thread| {
            let handle = store.handle();
            std::thread::spawn(move || {
                for i in 0..25 {
                    handle
                        .call_blocking(move |db| {
                            create_collection(
                                db,
                                &format!("t{thread}-{i:02}"),
                                "",
                                &Variables::new(),
                            )
                        })
                        .unwrap();
                }
            })
        })
        .collect();
    for handle in handles {
        handle.join().unwrap();
    }

    let all = store.call_blocking(|db| collections(db)).unwrap();
    assert_eq!(all.len(), 100);
    for thread in 0..4 {
        let prefix = format!("t{thread}-");
        let ids: Vec<i64> = {
            let mut mine: Vec<_> = all.iter().filter(|c| c.name.starts_with(&prefix)).collect();
            mine.sort_by(|a, b| a.name.cmp(&b.name));
            mine.iter().map(|c| c.id.0).collect()
        };
        assert!(
            ids.windows(2).all(|w| w[0] < w[1]),
            "thread {thread} ran out of order"
        );
    }
}

#[tokio::test]
async fn async_calls_work_from_a_runtime() {
    let (_dir, store) = store();
    let handle = store.handle();

    let created = handle
        .call(|db| create_collection(db, "async", "", &Variables::new()))
        .await
        .unwrap();

    assert_eq!(created.name, "async");
}

#[test]
fn closing_finishes_queued_work_and_later_calls_fail() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("poster.db");
    let store = Store::open(&path).unwrap();
    let handle = store.handle();
    let pending = {
        let handle = handle.clone();
        std::thread::spawn(move || {
            handle.call_blocking(|db| create_collection(db, "queued", "", &Variables::new()))
        })
    };
    pending.join().unwrap().unwrap();

    store.close().unwrap();

    assert!(matches!(
        handle.call_blocking(|db| collections(db)),
        Err(StoreError::Closed)
    ));
    let reopened = Store::open(&path).unwrap();
    assert_eq!(
        reopened.call_blocking(|db| collections(db)).unwrap()[0].name,
        "queued"
    );
}
