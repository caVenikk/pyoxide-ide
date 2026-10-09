//! The command registry.
//!
//! Every user-visible action is a [`Command`] with a stable dotted id
//! (`db.query`, `db.profile.list`). The UI, the CLI and the JSON control
//! channel used by agents all invoke commands through a [`Registry`], so they
//! share one implementation. Arguments and results are JSON values; typed
//! Rust functions are adapted with [`typed`].

use std::collections::BTreeMap;
use std::future::Future;
use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// Static description of a command, listed by `Registry::list`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandMeta {
    /// Stable dotted id, `area.verb` or `area.noun.verb`.
    pub id: &'static str,
    /// Short title for menus and the command palette.
    pub title: &'static str,
    /// One sentence on what it does.
    pub summary: &'static str,
    /// Example arguments as JSON, for help output and agents.
    pub args_example: &'static str,
}

/// Failure of a command. `code` is stable and machine-readable
/// (`invalid_args`, `not_found`, `db.auth`); `message` is for people and
/// never contains secrets.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[error("{message}")]
pub struct CommandError {
    pub code: String,
    pub message: String,
}

impl CommandError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn invalid_args(message: impl Into<String>) -> Self {
        Self::new("invalid_args", message)
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new("not_found", message)
    }
}

pub type CommandResult = Result<Json, CommandError>;

#[async_trait]
pub trait Command: Send + Sync {
    fn meta(&self) -> &CommandMeta;
    async fn invoke(&self, args: Json) -> CommandResult;
}

/// All registered commands, keyed by id.
#[derive(Default, Clone)]
pub struct Registry {
    commands: BTreeMap<&'static str, Arc<dyn Command>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a command. Panics on a duplicate id: that is a programming error.
    pub fn register(&mut self, command: Arc<dyn Command>) {
        let id = command.meta().id;
        assert!(
            self.commands.insert(id, command).is_none(),
            "duplicate command id {id}"
        );
    }

    pub fn get(&self, id: &str) -> Option<&Arc<dyn Command>> {
        self.commands.get(id)
    }

    /// Metadata of every command, sorted by id.
    pub fn list(&self) -> Vec<&CommandMeta> {
        self.commands.values().map(|c| c.meta()).collect()
    }

    pub async fn invoke(&self, id: &str, args: Json) -> CommandResult {
        let command = self
            .get(id)
            .ok_or_else(|| CommandError::not_found(format!("unknown command {id}")))?;
        command.invoke(args).await
    }
}

/// Adapt a typed async function into a [`Command`]. `A` is deserialized from
/// the JSON arguments (`null` counts as `{}`), `R` is serialized back.
pub fn typed<A, R, F, Fut>(meta: CommandMeta, f: F) -> Arc<dyn Command>
where
    A: DeserializeOwned + Send + 'static,
    R: Serialize + Send + 'static,
    F: Fn(A) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<R, CommandError>> + Send + 'static,
{
    Arc::new(Typed {
        meta,
        f,
        _types: PhantomData,
    })
}

struct Typed<A, R, F> {
    meta: CommandMeta,
    f: F,
    _types: PhantomData<fn(A) -> R>,
}

#[async_trait]
impl<A, R, F, Fut> Command for Typed<A, R, F>
where
    A: DeserializeOwned + Send + 'static,
    R: Serialize + Send + 'static,
    F: Fn(A) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<R, CommandError>> + Send + 'static,
{
    fn meta(&self) -> &CommandMeta {
        &self.meta
    }

    async fn invoke(&self, args: Json) -> CommandResult {
        let args = if args.is_null() {
            Json::Object(Default::default())
        } else {
            args
        };
        let args: A = serde_json::from_value(args)
            .map_err(|e| CommandError::invalid_args(format!("{}: {e}", self.meta.id)))?;
        let out = (self.f)(args).await?;
        serde_json::to_value(out).map_err(|e| CommandError::new("internal", e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct AddArgs {
        a: i64,
        #[serde(default)]
        b: i64,
    }

    fn registry() -> Registry {
        let mut r = Registry::new();
        r.register(typed(
            CommandMeta {
                id: "test.add",
                title: "Add",
                summary: "Adds two numbers.",
                args_example: r#"{"a": 1, "b": 2}"#,
            },
            |args: AddArgs| async move { Ok::<_, CommandError>(args.a + args.b) },
        ));
        r
    }

    #[tokio::test]
    async fn invokes_typed_command() {
        let r = registry();
        let out = r
            .invoke("test.add", serde_json::json!({"a": 2, "b": 3}))
            .await
            .unwrap();
        assert_eq!(out, serde_json::json!(5));
        assert_eq!(r.list()[0].id, "test.add");
    }

    #[tokio::test]
    async fn reports_bad_args_and_unknown_ids() {
        let r = registry();
        let e = r
            .invoke("test.add", serde_json::json!({"b": 1}))
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_args");
        let e = r.invoke("nope", Json::Null).await.unwrap_err();
        assert_eq!(e.code, "not_found");
    }

    #[test]
    #[should_panic(expected = "duplicate")]
    fn rejects_duplicates() {
        let mut r = registry();
        r.register(registry().get("test.add").unwrap().clone());
    }
}
