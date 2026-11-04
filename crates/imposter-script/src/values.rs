//! What scripts see: `request`, `response` and `vars`. They share their state through
//! `Arc`s, so a copy a script makes (`let r = request;`) changes the same request.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use imposter_core::execution::Response;
use imposter_core::model::{BodyType, ParamItem, RequestTemplate, Variables};
use imposter_core::{select, url};
use rhai::{Array, Dynamic, Engine, EvalAltResult, ImmutableString, Map};

type Result<T> = std::result::Result<T, Box<EvalAltResult>>;

pub fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    // A script error never leaves the data half-written, so a poisoned lock is still usable.
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn json(value: &serde_json::Value) -> Result<Dynamic> {
    rhai::serde::to_dynamic(value)
}

fn parse_json(text: &[u8], truncated: bool) -> Result<Dynamic> {
    let value = select::parse_body(text, truncated).map_err(|e| e.to_string())?;
    json(&value)
}

/// The request: editable before sending, read-only after.
#[derive(Clone)]
pub struct ScriptRequest {
    template: Arc<Mutex<RequestTemplate>>,
    editable: bool,
}

impl ScriptRequest {
    pub fn new(template: RequestTemplate, editable: bool) -> Self {
        Self {
            template: Arc::new(Mutex::new(template)),
            editable,
        }
    }

    pub fn template(&self) -> RequestTemplate {
        lock(&self.template).clone()
    }

    fn edit<T>(&mut self, change: impl FnOnce(&mut RequestTemplate) -> Result<T>) -> Result<T> {
        if !self.editable {
            return Err(
                "The request was already sent: change it in the before-sending script"
                    .to_owned()
                    .into(),
            );
        }
        change(&mut lock(&self.template))
    }

    fn read<T>(&self, read: impl FnOnce(&RequestTemplate) -> T) -> T {
        read(&lock(&self.template))
    }
}

fn is_token(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b))
}

fn header_name(name: &str) -> Result<()> {
    if is_token(name) {
        Ok(())
    } else {
        Err(format!("{name:?} is not a valid header name").into())
    }
}

fn find<'a>(items: &'a [ParamItem], name: &str, case: bool) -> Option<&'a ParamItem> {
    items.iter().find(|item| {
        item.is_enabled
            && if case {
                item.name == name
            } else {
                item.name.eq_ignore_ascii_case(name)
            }
    })
}

fn remove(items: &mut Vec<ParamItem>, name: &str, case: bool) {
    items.retain(|item| {
        !(item.is_enabled
            && if case {
                item.name == name
            } else {
                item.name.eq_ignore_ascii_case(name)
            })
    });
}

/// Query rows, made from the URL when there are none, so a change is made in one place;
/// the URL's query is then rewritten from the enabled rows, as the editor keeps them.
fn edit_query(template: &mut RequestTemplate, change: impl FnOnce(&mut Vec<ParamItem>)) {
    let mut parts = url::split(&template.url);
    if template.queries.is_empty() {
        template.queries = url::parse_pairs(&parts.query)
            .into_iter()
            .map(|(name, value)| ParamItem::new(name, value))
            .collect();
    }
    change(&mut template.queries);
    parts.query = url::encode_pairs(
        template
            .queries
            .iter()
            .filter(|q| q.is_enabled)
            .map(|q| (q.name.as_str(), q.value.as_str())),
    );
    template.url = url::join_with_fragment(&parts);
}

const fn raw_body(body_type: BodyType) -> bool {
    matches!(
        body_type,
        BodyType::None | BodyType::Json | BodyType::Plain | BodyType::Text
    )
}

fn register_request(engine: &mut Engine) {
    engine
        .register_type_with_name::<ScriptRequest>("Request")
        .register_get("method", |r: &mut ScriptRequest| {
            r.read(|t| t.method.clone())
        })
        .register_set(
            "method",
            |r: &mut ScriptRequest, method: ImmutableString| -> Result<()> {
                if !is_token(&method) {
                    return Err(format!("{method:?} is not a valid HTTP method").into());
                }
                r.edit(|t| {
                    t.method = method.to_string();
                    Ok(())
                })
            },
        )
        .register_get("url", |r: &mut ScriptRequest| r.read(|t| t.url.clone()))
        .register_set(
            "url",
            |r: &mut ScriptRequest, value: ImmutableString| -> Result<()> {
                r.edit(|t| {
                    // The query now comes from the new URL, as typing it in the editor would.
                    t.url = value.to_string();
                    t.queries.clear();
                    Ok(())
                })
            },
        )
        .register_fn("header", |r: &mut ScriptRequest, name: &str| {
            r.read(|t| {
                find(&t.headers, name, false).map_or(Dynamic::UNIT, |h| h.value.clone().into())
            })
        })
        .register_fn(
            "set_header",
            |r: &mut ScriptRequest, name: &str, value: &str| -> Result<()> {
                header_name(name)?;
                r.edit(|t| {
                    remove(&mut t.headers, name, false);
                    t.headers.push(ParamItem::new(name, value));
                    Ok(())
                })
            },
        )
        .register_fn(
            "add_header",
            |r: &mut ScriptRequest, name: &str, value: &str| -> Result<()> {
                header_name(name)?;
                r.edit(|t| {
                    t.headers.push(ParamItem::new(name, value));
                    Ok(())
                })
            },
        )
        .register_fn(
            "remove_header",
            |r: &mut ScriptRequest, name: &str| -> Result<()> {
                r.edit(|t| {
                    remove(&mut t.headers, name, false);
                    Ok(())
                })
            },
        )
        .register_fn("query", |r: &mut ScriptRequest, name: &str| {
            r.read(|t| {
                if t.queries.is_empty() {
                    url::parse_pairs(&url::split(&t.url).query)
                        .into_iter()
                        .find(|(n, _)| n == name)
                        .map_or(Dynamic::UNIT, |(_, v)| v.into())
                } else {
                    find(&t.queries, name, true).map_or(Dynamic::UNIT, |q| q.value.clone().into())
                }
            })
        })
        .register_fn(
            "set_query",
            |r: &mut ScriptRequest, name: &str, value: &str| -> Result<()> {
                r.edit(|t| {
                    edit_query(t, |rows| {
                        remove(rows, name, true);
                        rows.push(ParamItem::new(name, value));
                    });
                    Ok(())
                })
            },
        )
        .register_fn(
            "remove_query",
            |r: &mut ScriptRequest, name: &str| -> Result<()> {
                r.edit(|t| {
                    edit_query(t, |rows| remove(rows, name, true));
                    Ok(())
                })
            },
        )
        .register_get("body", |r: &mut ScriptRequest| {
            r.read(|t| {
                if raw_body(t.body_type) {
                    Dynamic::from(String::from_utf8_lossy(&t.body).into_owned())
                } else {
                    Dynamic::UNIT
                }
            })
        })
        .register_set(
            "body",
            |r: &mut ScriptRequest, value: ImmutableString| -> Result<()> {
                r.edit(|t| {
                    if !raw_body(t.body_type) {
                        return Err(format!(
                            "A {} body cannot be set by a script",
                            t.body_type.label()
                        )
                        .into());
                    }
                    if t.body_type == BodyType::None {
                        t.body_type = BodyType::Plain;
                    }
                    t.body = value.as_bytes().to_vec();
                    Ok(())
                })
            },
        )
        .register_fn("json", |r: &mut ScriptRequest| -> Result<Dynamic> {
            let (body, raw) = r.read(|t| (t.body.clone(), raw_body(t.body_type)));
            if !raw {
                return Err("Only a JSON or text body can be read as JSON"
                    .to_owned()
                    .into());
            }
            parse_json(&body, false)
        });
}

/// The response, read-only.
#[derive(Clone)]
pub struct ScriptResponse(pub Arc<Response>);

fn register_response(engine: &mut Engine) {
    engine
        .register_type_with_name::<ScriptResponse>("Response")
        .register_get("status", |r: &mut ScriptResponse| i64::from(r.0.status))
        .register_get("reason", |r: &mut ScriptResponse| r.0.reason.clone())
        .register_get("body", |r: &mut ScriptResponse| {
            String::from_utf8_lossy(&r.0.body).into_owned()
        })
        .register_get("truncated", |r: &mut ScriptResponse| r.0.truncated)
        .register_get("size", |r: &mut ScriptResponse| {
            i64::try_from(r.0.body.len()).unwrap_or(i64::MAX)
        })
        .register_get("time_ms", |r: &mut ScriptResponse| {
            #[allow(clippy::cast_possible_truncation)] // whole milliseconds of a short exchange
            let millis = (r.0.elapsed * 1000.0).round() as i64;
            millis
        })
        .register_fn("header", |r: &mut ScriptResponse, name: &str| {
            let values: Vec<&str> =
                r.0.headers
                    .iter()
                    .filter(|(n, _)| n.eq_ignore_ascii_case(name))
                    .map(|(_, v)| v.as_str())
                    .collect();
            if values.is_empty() {
                Dynamic::UNIT
            } else {
                values.join(", ").into()
            }
        })
        .register_get("headers", |r: &mut ScriptResponse| {
            let mut map = Map::new();
            for (name, value) in &r.0.headers {
                let key = name.to_ascii_lowercase();
                let joined = match map.get(key.as_str()) {
                    Some(earlier) => format!("{earlier}, {value}"),
                    None => value.clone(),
                };
                map.insert(key.into(), joined.into());
            }
            map
        })
        .register_fn("json", |r: &mut ScriptResponse| {
            parse_json(&r.0.body, r.0.truncated)
        })
        .register_fn(
            "path",
            |r: &mut ScriptResponse, path: &str| -> Result<Dynamic> {
                let path = select::parse_path(path).map_err(|e| e.to_string())?;
                let document =
                    select::parse_body(&r.0.body, r.0.truncated).map_err(|e| e.to_string())?;
                let found = select::query(&document, &path);
                match found.as_slice() {
                    [] => Ok(Dynamic::UNIT),
                    [one] => json(&one.value),
                    many => Ok(Dynamic::from(
                        many.iter()
                            .map(|f| json(&f.value))
                            .collect::<Result<Array>>()?,
                    )),
                }
            },
        );
}

/// Variables: what the request resolved against, plus this run's `vars.set`.
#[derive(Clone)]
pub struct Vars {
    values: Arc<Variables>,
    pub sets: Arc<Mutex<Vec<(String, String)>>>,
}

impl Vars {
    pub fn new(values: Variables) -> Self {
        Self {
            values: Arc::new(values),
            sets: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

fn register_vars(engine: &mut Engine) {
    engine
        .register_type_with_name::<Vars>("Variables")
        .register_fn("get", |v: &mut Vars, name: &str| {
            let set = lock(&v.sets)
                .iter()
                .rev()
                .find(|(n, _)| n == name)
                .map(|(_, value)| value.clone());
            set.or_else(|| v.values.get(name).cloned())
                .map_or(Dynamic::UNIT, Dynamic::from)
        })
        .register_fn(
            "set",
            |v: &mut Vars, name: &str, value: Dynamic| -> Result<()> {
                let name = name.trim();
                if name.is_empty() || name.contains(['{', '}', '\n']) {
                    return Err(format!("{name:?} is not a valid variable name").into());
                }
                lock(&v.sets).push((name.to_owned(), value.to_string()));
                Ok(())
            },
        );
}

pub fn register(engine: &mut Engine) {
    register_request(engine);
    register_response(engine);
    register_vars(engine);
}
