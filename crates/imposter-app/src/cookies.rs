//! The cookie jars kept on this computer, one per collections folder and environment
//! (`store::cookies` holds their JSON): read before Send, filled from what its response
//! set, and listed, pruned and cleared on the Variables screen. Runs and the command line
//! use jars of their own and never write here.

use imposter_core::execution::SetCookie;
use imposter_core::ids::EnvironmentId;
use imposter_core::model::{ParamItem, RequestTemplate};
use imposter_http::cookies::{Jar, StoredCookie};
use imposter_store::{StoreError, StoreHandle, Tx};

/// The jar for `environment` (none: requests sent without one) in the open folder. A jar
/// that does not read is taken as empty, and the next cookie kept replaces it: it held
/// cookies a server will set again.
pub fn read(db: &Tx<'_>, environment: Option<EnvironmentId>) -> Result<Jar, StoreError> {
    let text = imposter_store::cookies::read(db, environment)?;
    Ok(Jar::from_json(&text).unwrap_or_else(|error| {
        tracing::warn!(%error, "an unreadable cookie jar was taken as empty");
        Jar::default()
    }))
}

/// Keep what a response set in the stored jar, merged with what is there now, so two sends
/// at once both keep theirs.
pub fn keep(
    db: &Tx<'_>,
    environment: Option<EnvironmentId>,
    set: &[SetCookie],
) -> Result<(), StoreError> {
    let mut jar = read(db, environment)?;
    let mut changed = false;
    for cookie in set {
        changed |= jar.receive(&cookie.url, &cookie.header);
    }
    if changed {
        imposter_store::cookies::write(db, environment, &jar.to_json())?;
    }
    Ok(())
}

/// The cookies kept for `environment`, by domain, path and name.
pub async fn list(
    store: &StoreHandle,
    environment: Option<EnvironmentId>,
) -> Result<Vec<StoredCookie>, StoreError> {
    store
        .call(move |db| Ok(read(db, environment)?.cookies()))
        .await
}

/// Remove the cookies named by domain, path and name; the rest stay.
pub async fn remove(
    store: &StoreHandle,
    environment: Option<EnvironmentId>,
    cookies: Vec<StoredCookie>,
) -> Result<(), StoreError> {
    store
        .call(move |db| {
            let mut jar = read(db, environment)?;
            for cookie in &cookies {
                jar.remove(&cookie.domain, &cookie.path, &cookie.name);
            }
            imposter_store::cookies::write(db, environment, &jar.to_json())
        })
        .await
}

/// Remove every cookie kept for `environment`.
pub async fn clear(
    store: &StoreHandle,
    environment: Option<EnvironmentId>,
) -> Result<(), StoreError> {
    store
        .call(move |db| imposter_store::cookies::write(db, environment, ""))
        .await
}

/// Put the jar's `cookies` into `template` as sent, so History, the request as sent and
/// Copy as curl show them: after an enabled `Cookie` header when the request has one,
/// else as cookie rows. A name the request already sends is left to the request.
pub fn add_to(template: &mut RequestTemplate, cookies: Vec<(String, String)>) {
    let header = template
        .headers
        .iter()
        .position(|h| h.is_sent() && h.name.eq_ignore_ascii_case("cookie"));
    let mut sent: Vec<String> = template
        .cookies
        .iter()
        .filter(|c| c.is_sent())
        .map(|c| c.name.clone())
        .collect();
    if let Some(index) = header {
        sent.extend(
            template.headers[index]
                .value
                .split(';')
                .filter_map(|pair| pair.split_once('=').map(|(n, _)| n.trim().to_owned())),
        );
    }
    let added: Vec<(String, String)> = cookies
        .into_iter()
        .filter(|(name, _)| !sent.contains(name))
        .collect();
    if added.is_empty() {
        return;
    }
    match header {
        Some(index) => {
            let value = &mut template.headers[index].value;
            for (name, cookie) in added {
                if !value.trim().is_empty() {
                    value.push_str("; ");
                }
                value.push_str(&name);
                value.push('=');
                value.push_str(&cookie);
            }
        }
        None => template.cookies.extend(
            added
                .into_iter()
                .map(|(name, value)| ParamItem::new(name, value)),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
        list.iter()
            .map(|(n, v)| ((*n).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn the_jar_adds_rows_or_extends_the_cookie_header_and_never_overrides() {
        let mut rows = RequestTemplate {
            cookies: vec![ParamItem::new("session", "mine")],
            ..RequestTemplate::default()
        };
        add_to(&mut rows, pairs(&[("session", "jar"), ("theme", "dark")]));
        assert_eq!(
            rows.cookies,
            [
                ParamItem::new("session", "mine"),
                ParamItem::new("theme", "dark")
            ]
        );

        let mut header = RequestTemplate {
            headers: vec![ParamItem::new("Cookie", "session=mine")],
            ..RequestTemplate::default()
        };
        add_to(&mut header, pairs(&[("session", "jar"), ("theme", "dark")]));
        assert_eq!(header.headers[0].value, "session=mine; theme=dark");
        assert_eq!(header.cookies, []);

        let mut unchanged = RequestTemplate::default();
        add_to(&mut unchanged, Vec::new());
        assert_eq!(unchanged, RequestTemplate::default());
    }
}
