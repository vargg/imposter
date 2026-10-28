//! Workspace operations that combine several store calls in one transaction.

use poster_core::ids::{CollectionId, FolderId, RequestId};
use poster_core::model::{Request, RequestTemplate};
use poster_store::{StoreError, StoreHandle, collections};

/// A new request named `name`, last among its siblings, with the template defaults.
pub async fn create_request(
    store: &StoreHandle,
    collection: CollectionId,
    folder: Option<FolderId>,
    name: String,
) -> Result<Request, StoreError> {
    store
        .call(move |db| {
            let position = collections::next_position(db, collection, folder)?;
            collections::create_request(
                db,
                collection,
                folder,
                &RequestTemplate {
                    name,
                    position,
                    ..RequestTemplate::default()
                },
            )
        })
        .await
}

/// Copy the current template or stored request into a fresh sibling, atomically choosing its name.
pub async fn duplicate_request(
    store: &StoreHandle,
    id: RequestId,
    edited: Option<RequestTemplate>,
) -> Result<Request, StoreError> {
    store
        .call(move |db| {
            let source = collections::request(db, id)?.ok_or(StoreError::NotFound("request"))?;
            let collection = source
                .collection_id
                .ok_or(StoreError::NotFound("collection"))?;
            let tree =
                collections::tree(db, collection)?.ok_or(StoreError::NotFound("collection"))?;
            let mut template = edited.unwrap_or(source.template);
            let original = template.name.clone();
            let mut number = 1;
            loop {
                template.name = if number == 1 {
                    format!("{original} (copy)")
                } else {
                    format!("{original} (copy {number})")
                };
                if !tree
                    .requests
                    .iter()
                    .any(|r| r.folder_id == source.folder_id && r.name == template.name)
                {
                    break;
                }
                number += 1;
            }
            template.position = collections::next_position(db, collection, source.folder_id)?;
            collections::create_request(db, collection, source.folder_id, &template)
        })
        .await
}

/// A new folder, last among its siblings.
pub async fn create_folder(
    store: &StoreHandle,
    collection: CollectionId,
    parent: Option<FolderId>,
    name: String,
) -> Result<collections::Folder, StoreError> {
    store
        .call(move |db| {
            let position = collections::next_position(db, collection, parent)?;
            collections::create_folder(db, collection, parent, &name, "", position)
        })
        .await
}

/// Save an unsaved draft as a new request at the root of `collection`, named `name`.
pub async fn save_draft(
    store: &StoreHandle,
    draft: Request,
    collection: CollectionId,
    name: String,
) -> Result<Request, StoreError> {
    store
        .call(move |db| {
            let mut template = draft.template;
            template.name = name;
            template.position = collections::next_position(db, collection, None)?;
            let mut saved = collections::create_request(db, collection, None, &template)?;
            saved.collection_variables_snapshot = draft.collection_variables_snapshot;
            Ok(saved)
        })
        .await
}

#[cfg(test)]
mod tests {
    use poster_core::model::Variables;
    use poster_store::Store;

    use super::*;

    #[tokio::test]
    async fn duplicates_keep_fields_and_allocate_sibling_names_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("poster.db")).unwrap();
        let collection = store
            .call(|db| collections::create_collection(db, "API", "", &Variables::new()))
            .await
            .unwrap();
        let folder = create_folder(&store.handle(), collection.id, None, "Folder".into())
            .await
            .unwrap();
        let original = create_request(
            &store.handle(),
            collection.id,
            Some(folder.id),
            "Original".into(),
        )
        .await
        .unwrap();
        let id = original.id.unwrap();
        let mut edited = original.template.clone();
        edited.description = "notes".into();
        edited.url = "https://edited".into();
        edited.body = b"content".to_vec();
        let first = duplicate_request(&store.handle(), id, Some(edited.clone()))
            .await
            .unwrap();
        let second = duplicate_request(&store.handle(), id, None).await.unwrap();
        assert_eq!(first.template.name, "Original (copy)");
        assert_eq!(second.template.name, "Original (copy 2)");
        assert_eq!(
            (first.folder_id, first.collection_id),
            (original.folder_id, original.collection_id)
        );
        assert_ne!(first.id, original.id);
        edited.name.clone_from(&first.template.name);
        edited.position = first.template.position;
        assert_eq!(first.template, edited);
        assert!(second.template.position > first.template.position);
        assert_eq!(
            store
                .call(move |db| collections::request(db, id))
                .await
                .unwrap()
                .unwrap(),
            original
        );
        store
            .call(move |db| collections::delete_request(db, id))
            .await
            .unwrap();
        assert!(matches!(
            duplicate_request(&store.handle(), id, None).await,
            Err(StoreError::NotFound("request"))
        ));
    }

    #[tokio::test]
    async fn new_items_go_last_and_drafts_save_at_the_root() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("poster.db")).unwrap();
        let collection = store
            .call(|db| collections::create_collection(db, "API", "", &Variables::new()))
            .await
            .unwrap();

        let first = create_request(&store.handle(), collection.id, None, "first".into())
            .await
            .unwrap();
        let folder = create_folder(&store.handle(), collection.id, None, "folder".into())
            .await
            .unwrap();
        let mut draft = Request::default();
        draft.template.url = "http://draft".into();
        let saved = save_draft(&store.handle(), draft, collection.id, "draft".into())
            .await
            .unwrap();

        assert_eq!(
            (
                first.template.position,
                folder.position,
                saved.template.position
            ),
            (0, 1, 2)
        );
        assert_eq!(
            (saved.folder_id, saved.template.name.as_str()),
            (None, "draft")
        );
        let stored = store
            .call(move |db| collections::request(db, saved.id.unwrap()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.template.url, "http://draft");
    }
}
