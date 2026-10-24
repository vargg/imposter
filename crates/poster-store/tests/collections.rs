//! Collections, folders and requests.
#![cfg(test)]

mod support;

use poster_core::ids::{CollectionId, FolderId, RequestId};
use poster_core::model::{BodyType, ParamItem, RequestAuth, Variables};
use poster_store::collections::*;
use poster_store::{Conflict, StoreError};
use support::{store, template, vars};

#[test]
fn collections_round_trip_with_ordered_variables() {
    let (_dir, store) = store();
    let variables = vars(&[("zeta", "1"), ("alpha", ""), ("mid", "x")]);

    let created = store
        .call_blocking({
            let variables = variables.clone();
            move |db| create_collection(db, "API", "desc", &variables)
        })
        .unwrap();

    let read = store
        .call_blocking(move |db| collection(db, created.id))
        .unwrap()
        .unwrap();
    assert_eq!(read, created);
    assert_eq!(
        read.variables.keys().collect::<Vec<_>>(),
        ["zeta", "alpha", "mid"]
    );
}

#[test]
fn names_must_not_be_empty() {
    let (_dir, store) = store();
    let result = store.call_blocking(|db| create_collection(db, "  ", "", &Variables::new()));
    assert!(matches!(
        result,
        Err(StoreError::Conflict(Conflict::EmptyName))
    ));
}

#[test]
fn collections_are_listed_by_name() {
    let (_dir, store) = store();
    store
        .call_blocking(|db| {
            for name in ["b", "a", "c"] {
                create_collection(db, name, "", &Variables::new())?;
            }
            Ok(())
        })
        .unwrap();
    let names: Vec<_> = store
        .call_blocking(|db| collections(db))
        .unwrap()
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(names, ["a", "b", "c"]);
}

#[test]
fn ids_are_never_reused_after_deletion() {
    let (_dir, store) = store();
    let ids = store
        .call_blocking(|db| {
            let first = create_collection(db, "one", "", &Variables::new())?;
            let request = create_request(db, first.id, None, &template("r"))?;
            delete_collection(db, first.id)?;
            let second = create_collection(db, "two", "", &Variables::new())?;
            let request2 = create_request(db, second.id, None, &template("r"))?;
            Ok((first.id, second.id, request.id, request2.id))
        })
        .unwrap();
    assert!(ids.1 > ids.0);
    assert!(ids.3 > ids.2);
}

#[test]
fn the_tree_loads_nesting_order_and_summaries() {
    let (_dir, store) = store();
    let (collection_id, outer, inner) = store
        .call_blocking(|db| {
            let c = create_collection(db, "API", "", &Variables::new())?;
            let outer = create_folder(db, c.id, None, "outer", "", 1)?;
            let inner = create_folder(db, c.id, Some(outer.id), "inner", "", 0)?;
            create_request(
                db,
                c.id,
                None,
                &poster_core::model::RequestTemplate {
                    position: 2,
                    method: "POST".into(),
                    ..template("root-late")
                },
            )?;
            create_request(
                db,
                c.id,
                None,
                &poster_core::model::RequestTemplate {
                    position: 0,
                    ..template("root-early")
                },
            )?;
            create_request(db, c.id, Some(inner.id), &template("deep"))?;
            Ok((c.id, outer.id, inner.id))
        })
        .unwrap();

    let loaded = store
        .call_blocking(move |db| tree(db, collection_id))
        .unwrap()
        .unwrap();

    let folders: Vec<_> = loaded
        .folders
        .iter()
        .map(|f| (f.name.as_str(), f.parent_id))
        .collect();
    assert_eq!(folders, [("inner", Some(outer)), ("outer", None)]);
    let requests: Vec<_> = loaded
        .requests
        .iter()
        .map(|r| (r.name.as_str(), r.method.as_str(), r.folder_id))
        .collect();
    assert_eq!(
        requests,
        [
            ("root-early", "GET", None),
            ("deep", "GET", Some(inner)),
            ("root-late", "POST", None)
        ]
    );
    assert_eq!(
        store
            .call_blocking(|db| tree(db, CollectionId(999)))
            .unwrap(),
        None
    );
}

#[test]
fn requests_store_every_template_field() {
    let (_dir, store) = store();
    let mut full = template("full");
    full.headers = vec![
        ParamItem::new("A", "1"),
        ParamItem::new("A", "2"),
        ParamItem::new("Empty", ""),
    ];
    full.auth = RequestAuth::digest("u", "p");
    full.body = vec![0, 159, 146, 150];
    full.body_type = BodyType::Binary;
    full.options.timeout = Some(2.5);
    let stored = full.clone();

    let created = store
        .call_blocking(move |db| {
            let c = create_collection(db, "API", "", &Variables::new())?;
            create_request(db, c.id, None, &stored)
        })
        .unwrap();
    let id = created.id.unwrap();

    let read = store
        .call_blocking(move |db| request(db, id))
        .unwrap()
        .unwrap();
    assert_eq!(read.template, full);
    assert_eq!(read.collection_id, created.collection_id);
}

#[test]
fn update_rename_and_reorder_keep_one_source_of_truth() {
    let (_dir, store) = store();
    let (collection_id, a, b) = store
        .call_blocking(|db| {
            let c = create_collection(db, "API", "", &Variables::new())?;
            let a = create_request(db, c.id, None, &template("a"))?.id.unwrap();
            let b = create_request(db, c.id, None, &template("b"))?.id.unwrap();
            Ok((c.id, a, b))
        })
        .unwrap();

    store
        .call_blocking(move |db| {
            let mut edited = template("a");
            edited.url = "https://changed".into();
            update_request(db, a, &edited)?;
            rename_request(db, a, "renamed")?;
            reorder(db, &[], &[b, a])
        })
        .unwrap();

    let read = store
        .call_blocking(move |db| request(db, a))
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            read.template.name.as_str(),
            read.template.url.as_str(),
            read.template.position
        ),
        ("renamed", "https://changed", 1)
    );
    let order: Vec<_> = store
        .call_blocking(move |db| tree(db, collection_id))
        .unwrap()
        .unwrap()
        .requests
        .into_iter()
        .map(|r| r.name)
        .collect();
    assert_eq!(order, ["b", "renamed"]);
}

#[test]
fn missing_rows_are_reported() {
    let (_dir, store) = store();
    let missing = RequestId(42);
    for result in [
        store.call_blocking(move |db| update_request(db, missing, &template("x"))),
        store.call_blocking(move |db| rename_request(db, missing, "x")),
        store.call_blocking(move |db| delete_request(db, missing)),
        store.call_blocking(move |db| move_request(db, missing, None)),
        store.call_blocking(|db| rename_folder(db, FolderId(1), "x")),
        store.call_blocking(|db| delete_folder(db, FolderId(1))),
        store.call_blocking(|db| move_folder(db, FolderId(1), None)),
        store.call_blocking(|db| rename_collection(db, CollectionId(1), "x")),
        store.call_blocking(|db| delete_collection(db, CollectionId(1))),
        store.call_blocking(|db| set_collection_variables(db, CollectionId(1), &Variables::new())),
        store.call_blocking(|db| {
            create_request(db, CollectionId(1), None, &template("x")).map(drop)
        }),
        store.call_blocking(|db| create_folder(db, CollectionId(1), None, "x", "", 0).map(drop)),
    ] {
        assert!(matches!(result, Err(StoreError::NotFound(_))), "{result:?}");
    }
}

#[test]
fn deleting_cascades_down_the_tree() {
    let (_dir, store) = store();
    let (c, outer, deep_request) = store
        .call_blocking(|db| {
            let c = create_collection(db, "API", "", &Variables::new())?;
            let outer = create_folder(db, c.id, None, "outer", "", 0)?;
            let inner = create_folder(db, c.id, Some(outer.id), "inner", "", 0)?;
            let r = create_request(db, c.id, Some(inner.id), &template("deep"))?;
            create_request(db, c.id, None, &template("root"))?;
            Ok((c.id, outer.id, r.id.unwrap()))
        })
        .unwrap();

    store
        .call_blocking(move |db| delete_folder(db, outer))
        .unwrap();

    let loaded = store.call_blocking(move |db| tree(db, c)).unwrap().unwrap();
    assert_eq!(loaded.folders, []);
    assert_eq!(
        loaded
            .requests
            .iter()
            .map(|r| r.name.as_str())
            .collect::<Vec<_>>(),
        ["root"]
    );
    assert_eq!(
        store
            .call_blocking(move |db| request(db, deep_request))
            .unwrap(),
        None
    );

    store
        .call_blocking(move |db| delete_collection(db, c))
        .unwrap();
    assert_eq!(store.call_blocking(move |db| requests(db, c)).unwrap(), []);
}

#[test]
fn folder_moves_reject_cycles_and_other_collections() {
    let (_dir, store) = store();
    let (a, b, c, foreign, other_collection_request) = store
        .call_blocking(|db| {
            let one = create_collection(db, "one", "", &Variables::new())?;
            let two = create_collection(db, "two", "", &Variables::new())?;
            let a = create_folder(db, one.id, None, "a", "", 0)?;
            let b = create_folder(db, one.id, Some(a.id), "b", "", 0)?;
            let c = create_folder(db, one.id, Some(b.id), "c", "", 0)?;
            let foreign = create_folder(db, two.id, None, "foreign", "", 0)?;
            let r = create_request(db, two.id, None, &template("r"))?;
            Ok((a.id, b.id, c.id, foreign.id, r.id.unwrap()))
        })
        .unwrap();

    let conflict = |result: Result<(), StoreError>| match result {
        Err(StoreError::Conflict(conflict)) => conflict,
        other => panic!("expected a conflict, got {other:?}"),
    };
    assert_eq!(
        conflict(store.call_blocking(move |db| move_folder(db, a, Some(a)))),
        Conflict::MoveIntoItself
    );
    assert_eq!(
        conflict(store.call_blocking(move |db| move_folder(db, a, Some(c)))),
        Conflict::MoveIntoItself
    );
    assert_eq!(
        conflict(store.call_blocking(move |db| move_folder(db, a, Some(foreign)))),
        Conflict::CrossCollection
    );
    assert_eq!(
        conflict(store.call_blocking(move |db| move_request(
            db,
            other_collection_request,
            Some(b)
        ))),
        Conflict::CrossCollection
    );
    assert_eq!(
        conflict(store.call_blocking(move |db| {
            create_folder(db, CollectionId(2), Some(a), "x", "", 0).map(drop)
        })),
        Conflict::ParentOutsideCollection
    );

    store
        .call_blocking(move |db| move_folder(db, c, None))
        .unwrap();
    store
        .call_blocking(move |db| move_folder(db, a, Some(c)))
        .unwrap();
    let parent = store
        .call_blocking(move |db| folder(db, a))
        .unwrap()
        .unwrap()
        .parent_id;
    assert_eq!(parent, Some(c));
}

#[test]
fn new_items_go_last_among_siblings() {
    let (_dir, store) = store();
    let positions = store
        .call_blocking(|db| {
            let c = create_collection(db, "API", "", &Variables::new())?;
            let empty = next_position(db, c.id, None)?;
            create_folder(db, c.id, None, "f", "", 0)?;
            create_request(
                db,
                c.id,
                None,
                &poster_core::model::RequestTemplate {
                    position: 4,
                    ..template("r")
                },
            )?;
            Ok((empty, next_position(db, c.id, None)?))
        })
        .unwrap();
    assert_eq!(positions, (0, 5));
}
