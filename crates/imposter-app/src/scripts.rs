//! The scripts that run around a request: its collection's, its folders' (outermost first)
//! and its own, in that order before sending and again after the response (Postman's
//! order). Each is compiled once; Send, Copy as curl and load runs all run them through
//! here, so they cannot drift apart.

use std::sync::Arc;

use imposter_core::execution::{Response, ScriptTest};
use imposter_core::model::{Request, RequestTemplate, Scripts, Variables};
use imposter_script::Compiled;
use tokio_util::sync::CancellationToken;

use crate::executor::{AFTER_SCRIPT, BEFORE_SCRIPT, ExecuteError, SCRIPT_TIME, script_host};

/// Scripts of one level, named for errors: `Collection "API"`, `Folder "Users"`, or empty
/// for the request itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Level {
    pub name: String,
    pub scripts: Scripts,
}

/// The levels around a request, outermost first; the request's own come last.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ScriptChain(pub Vec<Level>);

/// The words naming a level's script in errors: "Before-sending script" for the request,
/// `Folder "Users" before-sending script` for the others.
pub fn script_name(level: &str, after: bool) -> String {
    match (level.is_empty(), after) {
        (true, false) => BEFORE_SCRIPT.to_owned(),
        (true, true) => AFTER_SCRIPT.to_owned(),
        (false, false) => format!("{level} before-sending script"),
        (false, true) => format!("{level} after-response script"),
    }
}

impl ScriptChain {
    /// The chain with `request`'s own scripts as the last level.
    #[must_use]
    pub fn with_request(&self, request: &Request) -> Self {
        let mut levels = self.0.clone();
        levels.push(Level {
            name: String::new(),
            scripts: request.template.scripts.clone(),
        });
        Self(levels)
    }

    /// The levels that have scripts, by name, for the editor to list.
    pub fn named(&self) -> Vec<&str> {
        self.0
            .iter()
            .filter(|level| !level.name.is_empty() && !level.scripts.is_empty())
            .map(|level| level.name.as_str())
            .collect()
    }

    /// Compile every script once; one that does not compile stops a send as any failing
    /// before-sending script does.
    pub fn compile(&self) -> Result<CompiledChain, ExecuteError> {
        let compile = |source: &str, name: String| -> Result<Option<Compiled>, ExecuteError> {
            if source.trim().is_empty() {
                return Ok(None);
            }
            imposter_script::compile(source)
                .map(Some)
                .map_err(|error| ExecuteError::Script(error.describe(&name)))
        };
        let mut levels = Vec::new();
        for level in &self.0 {
            let before = compile(&level.scripts.before, script_name(&level.name, false))?;
            let after = compile(&level.scripts.after, script_name(&level.name, true))?;
            if before.is_some() || after.is_some() {
                levels.push(CompiledLevel {
                    name: level.name.clone(),
                    before,
                    after,
                });
            }
        }
        Ok(CompiledChain(Arc::new(levels)))
    }
}

struct CompiledLevel {
    name: String,
    before: Option<Compiled>,
    after: Option<Compiled>,
}

/// A chain compiled once, cheap to share between the requests of a load run.
#[derive(Clone)]
pub struct CompiledChain(Arc<Vec<CompiledLevel>>);

impl std::fmt::Debug for CompiledChain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "CompiledChain({} levels)", self.0.len())
    }
}

/// What the before-sending scripts made of the request.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Before {
    pub template: RequestTemplate,
    /// `vars.set` calls, in order.
    pub sets: Vec<(String, String)>,
    /// `secrets.set` calls, in order.
    pub secret_sets: Vec<(String, String)>,
    pub console: Vec<String>,
}

/// What the after-response scripts reported, all of them having run.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct After {
    pub tests: Vec<ScriptTest>,
    /// One line per script that failed, naming it.
    pub errors: Vec<String>,
    pub sets: Vec<(String, String)>,
    pub secret_sets: Vec<(String, String)>,
    pub console: Vec<String>,
}

impl CompiledChain {
    pub fn has_before(&self) -> bool {
        self.0.iter().any(|level| level.before.is_some())
    }

    pub fn has_after(&self) -> bool {
        self.0.iter().any(|level| level.after.is_some())
    }

    /// Run the before-sending scripts in order, each on what the one before left and with
    /// its variables, off the async workers; each may run [`SCRIPT_TIME`]. The first
    /// failure, naming its script, is the request's "not sent" reason.
    pub async fn before(
        &self,
        template: RequestTemplate,
        values: Variables,
        stop: CancellationToken,
    ) -> Result<Before, String> {
        if !self.has_before() {
            return Ok(Before {
                template,
                ..Before::default()
            });
        }
        let chain = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut done = Before {
                template,
                ..Before::default()
            };
            let mut values = values;
            for level in chain.0.iter() {
                let Some(script) = &level.before else {
                    continue;
                };
                let outcome = imposter_script::before_compiled(
                    script,
                    &mut done.template,
                    &values,
                    script_host(stop.clone(), SCRIPT_TIME),
                );
                done.console.extend(outcome.console);
                if let Some(error) = outcome.error {
                    return Err(error.describe(&script_name(&level.name, false)));
                }
                for (name, value) in outcome.sets {
                    values.insert(name.clone(), value.clone());
                    done.sets.push((name, value));
                }
                // A later script reads a secret set here as `secrets.get` would.
                for (name, value) in outcome.secret_sets {
                    values.insert(imposter_core::variables::secret_key(&name), value.clone());
                    done.secret_sets.push((name, value));
                }
            }
            Ok(done)
        })
        .await
        .unwrap_or_else(|_| Err(format!("{BEFORE_SCRIPT} failed unexpectedly")))
    }

    /// Run the after-response scripts in order; one that fails does not keep the others
    /// from running, so their tests are not lost.
    pub async fn after(
        &self,
        template: RequestTemplate,
        response: Response,
        values: Variables,
        stop: CancellationToken,
    ) -> After {
        if !self.has_after() {
            return After::default();
        }
        let chain = self.clone();
        tokio::task::spawn_blocking(move || {
            let mut done = After::default();
            let mut values = values;
            for level in chain.0.iter() {
                let Some(script) = &level.after else { continue };
                let outcome = imposter_script::after_compiled(
                    script,
                    &template,
                    &response,
                    &values,
                    script_host(stop.clone(), SCRIPT_TIME),
                );
                done.tests.extend(outcome.tests);
                done.console.extend(outcome.console);
                if let Some(error) = outcome.error {
                    done.errors
                        .push(error.describe(&script_name(&level.name, true)));
                }
                for (name, value) in outcome.sets {
                    values.insert(name.clone(), value.clone());
                    done.sets.push((name, value));
                }
                // A later script reads a secret set here as `secrets.get` would.
                for (name, value) in outcome.secret_sets {
                    values.insert(imposter_core::variables::secret_key(&name), value.clone());
                    done.secret_sets.push((name, value));
                }
            }
            done
        })
        .await
        .unwrap_or_else(|_| After {
            errors: vec![format!("{AFTER_SCRIPT} failed unexpectedly")],
            ..After::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_are_named_by_their_level() {
        assert_eq!(script_name("", false), "Before-sending script");
        assert_eq!(script_name("", true), "After-response script");
        assert_eq!(
            script_name("Folder \"Users\"", false),
            "Folder \"Users\" before-sending script"
        );
        assert_eq!(
            script_name("Collection \"API\"", true),
            "Collection \"API\" after-response script"
        );
    }

    #[test]
    fn a_script_that_does_not_compile_is_named() {
        let chain = ScriptChain(vec![Level {
            name: "Folder \"Users\"".into(),
            scripts: Scripts {
                before: "let = ;".into(),
                after: String::new(),
            },
        }]);
        let error = chain.compile().unwrap_err().to_string();
        assert!(
            error.starts_with("Folder \"Users\" before-sending script, line 1"),
            "{error}"
        );
        let named = ScriptChain(vec![
            Level {
                name: "Collection \"API\"".into(),
                scripts: Scripts::default(),
            },
            Level {
                name: "Folder \"Users\"".into(),
                scripts: Scripts {
                    before: "1;".into(),
                    after: String::new(),
                },
            },
        ]);
        assert_eq!(
            named.named(),
            ["Folder \"Users\""],
            "levels without scripts are left out"
        );
    }
}
