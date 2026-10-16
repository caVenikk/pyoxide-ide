//! The messages of the Debug Adapter Protocol this client sends and reads,
//! as serde structs (the DAP specification,
//! https://microsoft.github.io/debug-adapter-protocol/specification). They
//! are private to the crate (ADR 0015); [`crate::types`] is what users see.
//! Members the client does not use are left out and ignored when read;
//! adapter-specific values stay `serde_json::Value`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types;

// ---- requests the client sends ---------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeArguments<'a> {
    #[serde(rename = "clientID")]
    pub client_id: &'a str,
    pub client_name: &'a str,
    #[serde(rename = "adapterID")]
    pub adapter_id: &'a str,
    pub locale: &'a str,
    pub lines_start_at1: bool,
    pub columns_start_at1: bool,
    pub path_format: &'a str,
    pub supports_variable_type: bool,
    pub supports_variable_paging: bool,
    pub supports_run_in_terminal_request: bool,
    pub supports_memory_references: bool,
    pub supports_progress_reporting: bool,
    pub supports_invalidated_event: bool,
    pub supports_args_can_be_interpreted_by_shell: bool,
    pub supports_start_debugging_request: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetBreakpointsArguments<'a> {
    pub source: Source,
    pub breakpoints: Vec<SourceBreakpoint<'a>>,
    pub lines: Vec<u32>,
    pub source_modified: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceBreakpoint<'a> {
    pub line: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hit_condition: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub log_message: Option<&'a str>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetExceptionBreakpointsArguments<'a> {
    pub filters: Vec<&'a str>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub filter_options: Vec<ExceptionFilterOptions<'a>>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExceptionFilterOptions<'a> {
    pub filter_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<&'a str>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StackTraceArguments {
    pub thread_id: i64,
    pub start_frame: usize,
    pub levels: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VariablesArguments {
    pub variables_reference: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EvaluateArguments<'a> {
    pub expression: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_id: Option<i64>,
    pub context: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetVariableArguments<'a> {
    pub variables_reference: i64,
    pub name: &'a str,
    pub value: &'a str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetExpressionArguments<'a> {
    pub expression: &'a str,
    pub value: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame_id: Option<i64>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadArguments {
    pub thread_id: i64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisconnectArguments {
    pub restart: bool,
    pub terminate_debuggee: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelArguments {
    pub request_id: i64,
}

// ---- answers and events the client reads -------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Capabilities {
    pub supports_configuration_done_request: Option<bool>,
    pub supports_conditional_breakpoints: Option<bool>,
    pub supports_hit_conditional_breakpoints: Option<bool>,
    pub supports_log_points: Option<bool>,
    pub supports_evaluate_for_hovers: Option<bool>,
    pub supports_set_variable: Option<bool>,
    pub supports_set_expression: Option<bool>,
    pub supports_terminate_request: Option<bool>,
    pub support_terminate_debuggee: Option<bool>,
    pub supports_cancel_request: Option<bool>,
    pub supports_exception_info_request: Option<bool>,
    pub supports_exception_filter_options: Option<bool>,
    pub supports_delayed_stack_trace_loading: Option<bool>,
    pub supports_clipboard_context: Option<bool>,
    pub supports_step_in_targets_request: Option<bool>,
    pub supports_restart_frame: Option<bool>,
    pub supports_goto_targets_request: Option<bool>,
    pub supports_single_thread_execution_requests: Option<bool>,
    pub exception_breakpoint_filters: Option<Vec<ExceptionBreakpointsFilter>>,
}

impl Capabilities {
    /// Sets in `to` what this (a `capabilities` event: only what changed)
    /// names.
    pub fn merge_into(self, to: &mut types::Capabilities) {
        macro_rules! merge {
            ($($from:ident => $to:ident),* $(,)?) => {
                $(if let Some(value) = self.$from { to.$to = value; })*
            };
        }
        merge!(
            supports_configuration_done_request => configuration_done,
            supports_conditional_breakpoints => conditional_breakpoints,
            supports_hit_conditional_breakpoints => hit_conditional_breakpoints,
            supports_log_points => log_points,
            supports_evaluate_for_hovers => evaluate_for_hovers,
            supports_set_variable => set_variable,
            supports_set_expression => set_expression,
            supports_terminate_request => terminate,
            support_terminate_debuggee => terminate_debuggee,
            supports_cancel_request => cancel,
            supports_exception_info_request => exception_info,
            supports_exception_filter_options => exception_filter_options,
            supports_delayed_stack_trace_loading => delayed_stack_trace_loading,
            supports_clipboard_context => clipboard_context,
            supports_step_in_targets_request => step_in_targets,
            supports_restart_frame => restart_frame,
            supports_goto_targets_request => goto_targets,
            supports_single_thread_execution_requests => single_thread_execution,
        );
        if let Some(filters) = self.exception_breakpoint_filters {
            to.exception_filters = filters
                .into_iter()
                .map(|f| types::ExceptionFilter {
                    id: f.filter,
                    label: f.label,
                    description: f.description,
                    default: f.default,
                    supports_condition: f.supports_condition,
                })
                .collect();
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExceptionBreakpointsFilter {
    pub filter: String,
    pub label: String,
    pub description: Option<String>,
    pub default: bool,
    pub supports_condition: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SourceRead {
    pub name: Option<String>,
    pub path: Option<String>,
    pub source_reference: Option<i64>,
    pub presentation_hint: Option<String>,
}

impl From<SourceRead> for types::Source {
    fn from(s: SourceRead) -> Self {
        types::Source {
            name: s.name,
            path: s.path.map(Into::into),
            reference: s.source_reference.filter(|r| *r > 0),
            deemphasized: s.presentation_hint.as_deref() == Some("deemphasize"),
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Breakpoint {
    pub id: Option<i64>,
    pub verified: bool,
    pub message: Option<String>,
    pub source: Option<SourceRead>,
    pub line: Option<u32>,
    pub column: Option<u32>,
}

impl From<Breakpoint> for types::Breakpoint {
    fn from(b: Breakpoint) -> Self {
        types::Breakpoint {
            id: b.id,
            verified: b.verified,
            message: b.message,
            source: b.source.map(Into::into),
            line: b.line,
            column: b.column,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct SetBreakpointsBody {
    pub breakpoints: Vec<Breakpoint>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ThreadsBody {
    pub threads: Vec<Thread>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Thread {
    pub id: i64,
    pub name: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StackTraceBody {
    pub stack_frames: Vec<StackFrame>,
    pub total_frames: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StackFrame {
    pub id: i64,
    pub name: String,
    pub source: Option<SourceRead>,
    pub line: u32,
    pub column: u32,
    pub presentation_hint: Option<String>,
}

impl From<StackFrame> for types::StackFrame {
    fn from(f: StackFrame) -> Self {
        types::StackFrame {
            id: f.id,
            name: f.name,
            source: f.source.map(Into::into),
            line: f.line,
            column: f.column,
            presentation: match f.presentation_hint.as_deref() {
                Some("label") => types::FramePresentation::Label,
                Some("subtle") => types::FramePresentation::Subtle,
                _ => types::FramePresentation::Normal,
            },
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ScopesBody {
    pub scopes: Vec<Scope>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Scope {
    pub name: String,
    pub presentation_hint: Option<String>,
    pub variables_reference: i64,
    pub named_variables: Option<usize>,
    pub indexed_variables: Option<usize>,
    pub expensive: bool,
}

impl From<Scope> for types::Scope {
    fn from(s: Scope) -> Self {
        types::Scope {
            name: s.name,
            hint: s.presentation_hint,
            reference: s.variables_reference,
            named: s.named_variables,
            indexed: s.indexed_variables,
            expensive: s.expensive,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct VariablesBody {
    pub variables: Vec<Variable>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Variable {
    pub name: String,
    pub value: String,
    #[serde(rename = "type")]
    pub type_name: Option<String>,
    pub presentation_hint: Option<PresentationHint>,
    pub evaluate_name: Option<String>,
    pub variables_reference: i64,
    pub named_variables: Option<usize>,
    pub indexed_variables: Option<usize>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct PresentationHint {
    pub kind: Option<String>,
    pub attributes: Vec<String>,
    pub visibility: Option<String>,
}

impl From<PresentationHint> for types::Presentation {
    fn from(h: PresentationHint) -> Self {
        types::Presentation {
            kind: h.kind,
            attributes: h.attributes,
            visibility: h.visibility,
        }
    }
}

impl From<Variable> for types::Variable {
    fn from(v: Variable) -> Self {
        types::Variable {
            name: v.name,
            value: v.value,
            type_name: v.type_name.filter(|t| !t.is_empty()),
            presentation: v.presentation_hint.map(Into::into).unwrap_or_default(),
            evaluate_name: v.evaluate_name,
            reference: v.variables_reference,
            named: v.named_variables,
            indexed: v.indexed_variables,
        }
    }
}

/// The body of `evaluate`, `setVariable` and `setExpression` (`result` or
/// `value` holds the text).
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ValueBody {
    pub result: Option<String>,
    pub value: Option<String>,
    #[serde(rename = "type")]
    pub type_name: Option<String>,
    pub presentation_hint: Option<PresentationHint>,
    pub variables_reference: Option<i64>,
    pub named_variables: Option<usize>,
    pub indexed_variables: Option<usize>,
}

impl From<ValueBody> for types::Evaluated {
    fn from(b: ValueBody) -> Self {
        types::Evaluated {
            value: b.result.or(b.value).unwrap_or_default(),
            type_name: b.type_name.filter(|t| !t.is_empty()),
            presentation: b.presentation_hint.map(Into::into).unwrap_or_default(),
            reference: b.variables_reference.unwrap_or(0),
            named: b.named_variables,
            indexed: b.indexed_variables,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ContinueBody {
    pub all_threads_continued: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExceptionInfoBody {
    pub exception_id: String,
    pub description: Option<String>,
    pub break_mode: String,
    pub details: Option<ExceptionDetails>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExceptionDetails {
    pub message: Option<String>,
    pub type_name: Option<String>,
    pub full_type_name: Option<String>,
    pub stack_trace: Option<String>,
}

impl From<ExceptionInfoBody> for types::ExceptionInfo {
    fn from(b: ExceptionInfoBody) -> Self {
        let details = b.details.unwrap_or_default();
        types::ExceptionInfo {
            id: b.exception_id,
            description: b.description,
            break_mode: b.break_mode,
            message: details.message,
            type_name: details.full_type_name.or(details.type_name),
            stack_trace: details.stack_trace,
        }
    }
}

/// The `error` of an error answer's body.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ErrorMessage {
    pub format: String,
    pub variables: BTreeMap<String, String>,
}

impl ErrorMessage {
    /// The text with its `{name}` variables filled in.
    pub fn text(&self) -> String {
        let mut text = self.format.clone();
        for (name, value) in &self.variables {
            text = text.replace(&format!("{{{name}}}"), value);
        }
        text
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct ErrorBody {
    pub error: Option<ErrorMessage>,
}

// ---- events -------------------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StoppedBody {
    pub reason: String,
    pub description: Option<String>,
    pub thread_id: Option<i64>,
    pub preserve_focus_hint: bool,
    pub text: Option<String>,
    pub all_threads_stopped: bool,
    pub hit_breakpoint_ids: Vec<i64>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ContinuedBody {
    pub thread_id: i64,
    pub all_threads_continued: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExitedBody {
    pub exit_code: i64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadBody {
    pub reason: String,
    pub thread_id: i64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OutputBody {
    pub category: Option<String>,
    pub output: String,
    pub source: Option<SourceRead>,
    pub line: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct BreakpointBody {
    pub reason: String,
    pub breakpoint: Breakpoint,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProcessBody {
    pub name: String,
    pub system_process_id: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct CapabilitiesBody {
    pub capabilities: Capabilities,
}

// ---- reverse requests ---------------------------------------------------------

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RunInTerminalArguments {
    pub kind: Option<String>,
    pub title: Option<String>,
    pub cwd: String,
    pub args: Vec<String>,
    pub env: BTreeMap<String, Option<String>>,
    pub args_can_be_interpreted_by_shell: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInTerminalResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_id: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shell_process_id: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct StartDebuggingArguments {
    pub configuration: Value,
    pub request: String,
}

/// Reads a message body (`null` or missing counts as `{}`).
pub fn body<T: for<'de> Deserialize<'de> + Default>(body: Value) -> Result<T, String> {
    if body.is_null() {
        return Ok(T::default());
    }
    serde_json::from_value(body).map_err(|e| e.to_string())
}
