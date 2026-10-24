//! Workspace operations that combine several store calls in one transaction.

use poster_core::ids::{CollectionId, FolderId};
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
