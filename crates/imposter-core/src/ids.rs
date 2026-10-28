//! Typed database identities, so a folder id can never be passed where a request id belongs.

use serde::{Deserialize, Serialize};

macro_rules! id {
    ($($(#[$doc:meta])* $name:ident),* $(,)?) => {$(
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub i64);

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    )*};
}

id!(
    CollectionId,
    FolderId,
    RequestId,
    EnvironmentId,
    ExecutionId
);
