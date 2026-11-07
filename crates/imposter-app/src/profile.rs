//! Running a request saved in the personal profile from outside the app: the command line
//! names it by its breadcrumb (`Shop API / Users / Create user`) and gets the same inputs
//! Send would read, with three differences the user chose. The environment is the one named,
//! never the app's active one; a red (production) environment needs the app's
//! `allow_production_from_cli` setting; and captures and script variables stay in the run.

use imposter_core::model::{EnvColor, Variables};
use imposter_store::{StoreError, StoreHandle, collections, environments, kv};

use crate::document::{DocumentError, RequestPath, Selected};
use crate::executor::{EnvironmentChoice, Writes, read_inputs};

/// Why a request is chosen: running one may refuse production, inspecting one never does,
/// since it reveals only names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    Run,
    Inspect,
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error(transparent)]
    Path(#[from] DocumentError),
    #[error(
        "{name} is a production (red) environment. To use it from the command line, turn on \
         \"Allow production environments\" in the app's Settings → Command line"
    )]
    Production { name: String },
    #[error(transparent)]
    Store(#[from] StoreError),
}

fn path_of(request: &collections::IndexedRequest) -> RequestPath {
    let mut place: Vec<&str> = vec![request.collection.as_str()];
    place.extend(request.folders.iter().map(String::as_str));
    RequestPath::new(&place, &request.name)
}

fn paths(db: &imposter_store::Tx<'_>) -> Result<Vec<String>, StoreError> {
    Ok(collections::request_index(db)?
        .iter()
        .map(|request| path_of(request).to_string())
        .collect())
}

/// Every saved request's path, collection first, as the command line names them.
pub async fn request_paths(store: &StoreHandle) -> Result<Vec<String>, StoreError> {
    store.call(paths).await
}

/// Choose the saved request at `path` and read its inputs in one database command, so the
/// run sees one consistent snapshot. `values` go above every scope.
pub async fn select(
    store: &StoreHandle,
    path: &RequestPath,
    environment: Option<&str>,
    values: &Variables,
    purpose: Purpose,
) -> Result<Selected, ProfileError> {
    let path = path.clone();
    let environment = environment.map(str::to_owned);
    let values = values.clone();
    store
        .call(move |db| {
            Ok(select_in(
                db,
                &path,
                environment.as_deref(),
                &values,
                purpose,
            ))
        })
        .await?
}

fn select_in(
    db: &imposter_store::Tx<'_>,
    path: &RequestPath,
    environment: Option<&str>,
    values: &Variables,
    purpose: Purpose,
) -> Result<Selected, ProfileError> {
    let index = collections::request_index(db)?;
    let matches: Vec<&collections::IndexedRequest> = index
        .iter()
        .filter(|request| {
            let names: Vec<String> = path_of(request).parts().to_vec();
            path.is(&names.iter().map(String::as_str).collect::<Vec<_>>())
        })
        .collect();
    let id = match matches.as_slice() {
        [one] => one.id,
        [] => {
            return Err(DocumentError::NoRequest {
                path: path.to_string(),
                available: paths(db)?,
            }
            .into());
        }
        several => {
            return Err(DocumentError::Ambiguous {
                path: path.to_string(),
                count: several.len(),
            }
            .into());
        }
    };
    let request = collections::request(db, id)?.ok_or_else(|| DocumentError::NoRequest {
        path: path.to_string(),
        available: Vec::new(),
    })?;
    let chosen = match environment {
        None => None,
        Some(name) => {
            let all = environments::environments(db)?;
            let Some(found) = all.iter().find(|e| e.name == name) else {
                return Err(DocumentError::NoEnvironment {
                    name: name.to_owned(),
                    available: all.iter().map(|e| e.name.clone()).collect(),
                }
                .into());
            };
            if purpose == Purpose::Run
                && found.color == Some(EnvColor::Red)
                && !kv::app_settings(db)?.allow_production_from_cli
            {
                return Err(ProfileError::Production {
                    name: found.name.clone(),
                });
            }
            Some(found.id)
        }
    };
    let mut inputs = read_inputs(db, &request, EnvironmentChoice::Chosen(chosen))?;
    inputs.writes = Writes::Run;
    inputs.snapshot = inputs.snapshot.with_inputs(values);
    Ok(Selected {
        path: path.clone(),
        request,
        inputs,
    })
}
