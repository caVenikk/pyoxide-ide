//! The one-time import of the reference IDE's breakpoints and watches from
//! `.idea/workspace.xml` (component `XDebuggerManager`: `BreakpointManagerState`
//! and `WatchesManagerState` of IntelliJ Platform's `xdebugger-impl`). The
//! file is only read.
//!
//! ```xml
//! <component name="XDebuggerManager">
//!   <breakpoint-manager>
//!     <breakpoints>
//!       <line-breakpoint enabled="true" suspend="THREAD" type="python-line">
//!         <url>file://$PROJECT_DIR$/main.py</url>
//!         <line>6</line>
//!         <condition expression="x &gt; 1" language="Python" />
//!         <option name="timeStamp" value="1" />
//!       </line-breakpoint>
//!     </breakpoints>
//!     <default-breakpoints>
//!       <breakpoint type="python-exception">
//!         <properties notifyOnTerminate="true" exception="BaseException">
//!           <option name="notifyOnTerminate" value="true" />
//!         </properties>
//!       </breakpoint>
//!     </default-breakpoints>
//!   </breakpoint-manager>
//!   <watches-manager>
//!     <configuration name="PythonConfigurationType">
//!       <watch expression="x" language="Python" />
//!     </configuration>
//!   </watches-manager>
//! </component>
//! ```
//!
//! The reference writes lines from 0 and files as URLs with path variables;
//! attributes at their default value are left out (`enabled` is false
//! then, `suspend` is `ALL`). Which breakpoint types are imported is the
//! product's ([`IdeaTypes`]).

use std::path::Path;

use run_config::xml::{self, Element};

use crate::breakpoints::{
    BreakpointId, Breakpoints, ExceptionBreakpoint, LineBreakpoint, SuspendPolicy, Watches,
};

/// The reference's breakpoint and configuration types a product imports.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdeaTypes {
    /// Line breakpoint types (`python-line`).
    pub line: Vec<String>,
    /// Exception breakpoint types (`python-exception`).
    pub exception: Vec<String>,
    /// The property of an exception breakpoint that names its class
    /// (`exception`).
    pub exception_class: String,
    /// The reference's run configuration type ids and the product's
    /// (`PythonConfigurationType` to `python`): watches are kept per type.
    pub configuration_types: Vec<(String, String)>,
}

/// What the reference had.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Imported {
    pub breakpoints: Breakpoints,
    /// Exception breakpoints were found (default or not).
    pub has_exception_breakpoints: bool,
    pub watches: Watches,
}

/// Reads `.idea/workspace.xml` of `root`; `None` when the file is missing
/// or unreadable.
pub fn import(root: &Path, types: &IdeaTypes) -> Option<Imported> {
    let path = root.join(project::idea::IDEA_DIR).join("workspace.xml");
    let text = std::fs::read_to_string(path).ok()?;
    Some(read_workspace(&text, root, types))
}

/// Reads the debugger state of a `workspace.xml` text.
pub fn read_workspace(text: &str, root: &Path, types: &IdeaTypes) -> Imported {
    let mut out = Imported::default();
    let Ok(doc) = xml::parse(text) else {
        return out;
    };
    let Some(component) = doc
        .children_named("component")
        .find(|c| c.attr("name") == Some("XDebuggerManager"))
    else {
        return out;
    };
    if let Some(manager) = component.child("breakpoint-manager") {
        for list in manager.children_named("breakpoints") {
            for element in &list.children {
                read_breakpoint(element, false, root, types, &mut out);
            }
        }
        for list in manager.children_named("default-breakpoints") {
            for element in list.children_named("breakpoint") {
                read_breakpoint(element, true, root, types, &mut out);
            }
        }
    }
    if let Some(watches) = component.child("watches-manager") {
        for configuration in watches.children_named("configuration") {
            let Some(name) = configuration.attr("name") else {
                continue;
            };
            let key = types
                .configuration_types
                .iter()
                .find(|(reference, _)| reference == name)
                .map_or(name, |(_, own)| own.as_str());
            for watch in configuration.children_named("watch") {
                if let Some(expression) = watch.attr("expression") {
                    out.watches.add(key, expression, None);
                }
            }
        }
    }
    out
}

fn read_breakpoint(
    element: &Element,
    default: bool,
    root: &Path,
    types: &IdeaTypes,
    out: &mut Imported,
) {
    let Some(kind) = element.attr("type") else {
        return;
    };
    let enabled = element.bool_attr("enabled");
    let suspend = element
        .attr("suspend")
        .and_then(SuspendPolicy::parse)
        .unwrap_or_default();
    let condition = expression(element, "condition");
    if element.name == "line-breakpoint" && types.line.iter().any(|t| t == kind) {
        let Some(url) = element.child("url").map(|u| u.text.trim()) else {
            return;
        };
        let Some(line) = element
            .child("line")
            .and_then(|l| l.text.trim().parse::<u32>().ok())
        else {
            return;
        };
        let path = url.strip_prefix("file://").unwrap_or(url);
        let Some(file) = run_config::paths::resolve(path, root) else {
            return;
        };
        out.breakpoints.add_line(LineBreakpoint {
            id: BreakpointId(0),
            file,
            line: line + 1,
            enabled,
            suspend,
            condition,
            log_message: element.bool_attr("log-message"),
            log_stack: element.bool_attr("log-stack"),
            log_expression: expression(element, "log-expression"),
            temporary: element
                .field("temporary")
                .is_some_and(|v| v.eq_ignore_ascii_case("true")),
        });
    } else if element.name == "breakpoint" && types.exception.iter().any(|t| t == kind) {
        let mut properties = std::collections::BTreeMap::new();
        if let Some(element) = element.child("properties") {
            for (name, value) in &element.attrs {
                properties.insert(name.clone(), value.clone());
            }
            for option in element.children_named("option") {
                if let (Some(name), Some(value)) = (option.attr("name"), option.attr("value")) {
                    properties.insert(name.to_owned(), value.to_owned());
                }
            }
        }
        let Some(class) = properties.remove(&types.exception_class) else {
            return;
        };
        out.has_exception_breakpoints = true;
        out.breakpoints.add_exception(ExceptionBreakpoint {
            id: BreakpointId(0),
            kind: kind.to_owned(),
            class,
            default,
            enabled,
            suspend,
            condition,
            properties,
        });
    }
}

/// An expression element (`<condition expression=".." language=".."/>`,
/// `XExpressionState`); a disabled or empty one is none.
fn expression(element: &Element, name: &str) -> Option<String> {
    let e = element.child(name)?;
    if e.bool_attr("disabled") {
        return None;
    }
    let text = e
        .attr("expression")
        .map(str::to_owned)
        .or_else(|| e.child("expression").map(|x| x.text.clone()))?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn python_types() -> IdeaTypes {
        IdeaTypes {
            line: vec!["python-line".into()],
            exception: vec!["python-exception".into()],
            exception_class: "exception".into(),
            configuration_types: vec![("PythonConfigurationType".into(), "python".into())],
        }
    }

    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<project version="4">
  <component name="RunManager" selected="Python.main" />
  <component name="XDebuggerManager">
    <breakpoint-manager>
      <breakpoints>
        <line-breakpoint enabled="true" type="python-line">
          <url>file://$PROJECT_DIR$/main.py</url>
          <line>6</line>
          <option name="timeStamp" value="1" />
        </line-breakpoint>
        <line-breakpoint suspend="THREAD" type="python-line">
          <url>file://$PROJECT_DIR$/pkg/util.py</url>
          <line>0</line>
          <condition expression="x &gt; 1" language="Python" />
          <option name="timeStamp" value="2" />
        </line-breakpoint>
        <line-breakpoint enabled="true" suspend="NONE" type="python-line" log-message="true">
          <url>file://$PROJECT_DIR$/main.py</url>
          <line>9</line>
          <log-expression expression="items" language="Python" />
          <condition disabled="true" expression="y" language="Python" />
          <option name="temporary" value="true" />
        </line-breakpoint>
        <line-breakpoint enabled="true" type="java-line">
          <url>file://$PROJECT_DIR$/Main.java</url>
          <line>3</line>
        </line-breakpoint>
        <breakpoint enabled="true" type="python-exception">
          <properties notifyOnlyOnFirst="true" exception="ValueError">
            <option name="notifyOnlyOnFirst" value="true" />
          </properties>
        </breakpoint>
      </breakpoints>
      <default-breakpoints>
        <breakpoint type="python-exception">
          <properties notifyOnTerminate="true" exception="BaseException">
            <option name="notifyOnTerminate" value="true" />
          </properties>
        </breakpoint>
      </default-breakpoints>
    </breakpoint-manager>
    <watches-manager>
      <configuration name="PythonConfigurationType">
        <watch expression="x" language="Python" />
        <watch expression="len(items)" language="Python" />
      </configuration>
      <configuration name="Other">
        <watch expression="y" />
      </configuration>
    </watches-manager>
  </component>
</project>"#;

    #[test]
    fn reads_breakpoints_and_watches_of_a_sample_workspace() {
        let root = Path::new("/work/project");
        let imported = read_workspace(SAMPLE, root, &python_types());
        let b = &imported.breakpoints;
        assert_eq!(b.line().len(), 3, "{b:?}");
        let main = root.join("main.py");
        let first = b.at(&main, 7).unwrap();
        assert!(first.enabled);
        assert_eq!(first.suspend, SuspendPolicy::All);
        assert_eq!(first.condition, None);
        let util = b.at(&root.join("pkg").join("util.py"), 1).unwrap();
        assert!(!util.enabled);
        assert_eq!(util.suspend, SuspendPolicy::Thread);
        assert_eq!(util.condition.as_deref(), Some("x > 1"));
        let logging = b.at(&main, 10).unwrap();
        assert!(logging.logs_only() && logging.log_message && logging.temporary);
        assert_eq!(logging.log_expression.as_deref(), Some("items"));
        assert_eq!(logging.condition, None);

        assert!(imported.has_exception_breakpoints);
        let exceptions = b.exception();
        assert_eq!(exceptions.len(), 2);
        assert_eq!(exceptions[0].class, "ValueError");
        assert!(exceptions[0].enabled && !exceptions[0].default);
        assert!(exceptions[0].flag("notifyOnlyOnFirst"));
        assert_eq!(exceptions[1].class, "BaseException");
        assert!(exceptions[1].default && !exceptions[1].enabled);
        assert!(exceptions[1].flag("notifyOnTerminate"));

        assert_eq!(imported.watches.get("python"), ["x", "len(items)"]);
        assert_eq!(imported.watches.get("Other"), ["y"]);
    }

    #[test]
    fn nothing_without_the_component() {
        let imported = read_workspace(
            "<project><component name=\"RunManager\"/></project>",
            Path::new("/p"),
            &python_types(),
        );
        assert_eq!(imported, Imported::default());
        assert_eq!(
            read_workspace("not xml <", Path::new("/p"), &python_types()),
            Imported::default()
        );
    }
}
