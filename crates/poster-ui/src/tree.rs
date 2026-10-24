//! The collection tree as the sidebar shows it, built from one store `Tree`.

use std::collections::HashMap;

use poster_app::core::ids::{FolderId, RequestId};
use poster_app::store::collections::{Folder, RequestSummary, Tree};

/// Something in the tree that can be renamed, moved or deleted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeId {
    Folder(FolderId),
    Request(RequestId),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    Folder { folder: Folder, children: Vec<Node> },
    Request(RequestSummary),
}

impl Node {
    pub fn id(&self) -> NodeId {
        match self {
            Self::Folder { folder, .. } => NodeId::Folder(folder.id),
            Self::Request(request) => NodeId::Request(request.id),
        }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Folder { folder, .. } => &folder.name,
            Self::Request(request) => &request.name,
        }
    }

    fn sort_key(&self) -> (i64, i64) {
        match self {
            Self::Folder { folder, .. } => (folder.position, folder.id.0),
            Self::Request(request) => (request.position, request.id.0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeView {
    pub source: Tree,
    pub roots: Vec<Node>,
}

impl TreeView {
    /// Nest folders and requests by parent; siblings sort by position, then id, folders and
    /// requests mixed (as stored positions say).
    pub fn build(source: Tree) -> Self {
        let mut children: HashMap<Option<FolderId>, Vec<Node>> = HashMap::new();
        for request in &source.requests {
            children
                .entry(request.folder_id)
                .or_default()
                .push(Node::Request(request.clone()));
        }
        let roots = attach(&source.folders, None, &mut children);
        Self { source, roots }
    }

    pub fn request(&self, id: RequestId) -> Option<&RequestSummary> {
        self.source.requests.iter().find(|r| r.id == id)
    }

    pub fn folder(&self, id: FolderId) -> Option<&Folder> {
        self.source.folders.iter().find(|f| f.id == id)
    }

    fn folder_names(&self, mut parent: Option<FolderId>) -> Vec<String> {
        let mut path = Vec::new();
        while let Some(id) = parent {
            let Some(folder) = self.folder(id) else { break };
            path.insert(0, folder.name.clone());
            parent = folder.parent_id;
        }
        path
    }

    /// "Collection / Folder / Request" for the context bar.
    pub fn request_path(&self, id: RequestId) -> Option<String> {
        let request = self.request(id)?;
        let mut parts = vec![self.source.collection.name.clone()];
        parts.extend(self.folder_names(request.folder_id));
        parts.push(request.name.clone());
        Some(parts.join(" / "))
    }

    /// Where a node may move: the collection root and every folder, labelled by path.
    pub fn destinations(&self) -> Vec<(String, Option<FolderId>)> {
        let mut out = vec![("Collection root".to_owned(), None)];
        visit(&self.roots, self, &mut out);
        out
    }

    /// Every request inside `folder`, at any depth.
    pub fn requests_under(&self, folder: FolderId) -> Vec<RequestId> {
        let mut inside = vec![folder];
        let mut grew = true;
        while grew {
            grew = false;
            for candidate in &self.source.folders {
                if candidate.parent_id.is_some_and(|p| inside.contains(&p))
                    && !inside.contains(&candidate.id)
                {
                    inside.push(candidate.id);
                    grew = true;
                }
            }
        }
        self.source
            .requests
            .iter()
            .filter(|r| r.folder_id.is_some_and(|f| inside.contains(&f)))
            .map(|r| r.id)
            .collect()
    }
}

fn attach(
    folders: &[Folder],
    parent: Option<FolderId>,
    children: &mut HashMap<Option<FolderId>, Vec<Node>>,
) -> Vec<Node> {
    let mut nodes = children.remove(&parent).unwrap_or_default();
    for folder in folders.iter().filter(|f| f.parent_id == parent) {
        let inner = attach(folders, Some(folder.id), children);
        nodes.push(Node::Folder {
            folder: folder.clone(),
            children: inner,
        });
    }
    nodes.sort_by_key(Node::sort_key);
    nodes
}

fn visit(nodes: &[Node], view: &TreeView, out: &mut Vec<(String, Option<FolderId>)>) {
    for node in nodes {
        if let Node::Folder { folder, children } = node {
            let mut names = view.folder_names(folder.parent_id);
            names.push(folder.name.clone());
            let path = names.join(" / ");
            let mut label = path.clone();
            let mut suffix = 2;
            while out.iter().any(|(existing, _)| *existing == label) {
                label = format!("{path} ({suffix})");
                suffix += 1;
            }
            out.push((label, Some(folder.id)));
            visit(children, view, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use poster_app::core::ids::CollectionId;
    use poster_app::core::model::Variables;
    use poster_app::store::collections::Collection;

    use super::*;

    fn folder(id: i64, parent: Option<i64>, name: &str, position: i64) -> Folder {
        Folder {
            id: FolderId(id),
            collection_id: CollectionId(1),
            parent_id: parent.map(FolderId),
            name: name.into(),
            description: String::new(),
            position,
        }
    }

    fn request(id: i64, folder: Option<i64>, name: &str, position: i64) -> RequestSummary {
        RequestSummary {
            id: RequestId(id),
            folder_id: folder.map(FolderId),
            name: name.into(),
            method: "GET".into(),
            position,
        }
    }

    fn view() -> TreeView {
        TreeView::build(Tree {
            collection: Collection {
                id: CollectionId(1),
                name: "API".into(),
                description: String::new(),
                variables: Variables::new(),
            },
            folders: vec![
                folder(10, None, "Users", 1),
                folder(11, Some(10), "Admin", 0),
                folder(12, None, "Users", 3),
            ],
            requests: vec![
                request(1, None, "health", 0),
                request(2, Some(11), "promote", 0),
                request(3, None, "late", 2),
            ],
        })
    }

    #[test]
    fn siblings_mix_folders_and_requests_by_position() {
        let view = view();
        let names: Vec<&str> = view.roots.iter().map(Node::name).collect();
        assert_eq!(names, ["health", "Users", "late", "Users"]);
    }

    #[test]
    fn paths_destinations_and_contents() {
        let view = view();
        assert_eq!(
            view.request_path(RequestId(2)).unwrap(),
            "API / Users / Admin / promote"
        );
        assert_eq!(view.request_path(RequestId(99)), None);
        let labels: Vec<String> = view
            .destinations()
            .into_iter()
            .map(|(label, _)| label)
            .collect();
        assert_eq!(
            labels,
            ["Collection root", "Users", "Users / Admin", "Users (2)"]
        );
        assert_eq!(view.requests_under(FolderId(10)), [RequestId(2)]);
        assert_eq!(view.requests_under(FolderId(12)), []);
    }
}
