//! Commands of running files, the Run tool window and its console (ADR
//! 0007). Names and texts follow the reference IDE's actions (`Run`,
//! `RunClass`, `Rerun`, `Stop`, `CloseContent`, `PreviousOccurence`,
//! `NextOccurence`, `SendEOF`, the console toolbar).

ui::ui_commands! {
    COMMANDS, scope = "Workbench";
    /// Run the selected configuration: the current file.
    Run => "run.run", "Run",
        "Runs the selected run configuration; with Current File selected, the file in the editor.";
    /// Run the file in the editor.
    RunCurrentFile => "run.current-file", "Run Context Configuration",
        "Runs the file in the editor (the reference's context configuration).";
    /// Stop a run.
    Stop { tab: Option<usize> } => "run.stop", "Stop",
        "Stops the process of a Run tab (zero-based), or of the only running one; a second Stop kills it. With several running and no tab, offers them in a popup.",
        r#"{"tab": 0}"#;
    /// Stop every run.
    StopAll => "run.stop-all", "Stop All",
        "Stops the processes of all Run tabs.";
    /// Show the run configurations.
    ChooseConfiguration => "run.configurations", "Run / Debug Configurations",
        "Opens the run configuration dropdown of the main toolbar.";
    /// Select Current File.
    SelectCurrentFile => "run.select-current-file", "Current File",
        "Makes Current File the selected run configuration: Run runs the file in the editor.";
    /// Show more run actions.
    MoreActions => "run.more-actions", "More Actions",
        "Opens the run widget's More Actions popup.";
    /// Describe the runs.
    RunState => "run.state", "Run State",
        "Returns the Run tool window's tabs, their processes, consoles and the run widget as JSON.";
}

ui::ui_commands! {
    CONFIGURATION_COMMANDS, scope = "Workbench";
    /// Select a run configuration.
    SelectConfiguration { name: Option<String>, r#type: Option<String> }
        => "run.configuration.select", "Select Run Configuration",
        "Makes a run configuration the selected one, by name (and type id when two types have the name); without a name, Current File.",
        r#"{"name": "server"}"#;
    /// Run a run configuration.
    LaunchConfiguration { name: Option<String>, r#type: Option<String> }
        => "run.configuration.run", "Run Configuration",
        "Runs a run configuration by name (and type id when two types have the name), or the selected one.",
        r#"{"name": "server"}"#;
    /// Show or hide every configuration in the dropdown.
    ToggleAllConfigurations => "run.configurations.toggle-all", "All Configurations",
        "Shows or hides every run configuration below the recent ones in the run widget's dropdown, and opens the dropdown again.";
    /// Import the reference IDE's run configurations again.
    ImportReference => "run.configurations.import-reference", "Import Run Configurations from the Reference IDE",
        "Reads the reference IDE's run configurations of the project (.idea and *.run.xml) again and adds the ones that do not exist yet; .idea is never written.";
}

ui::ui_commands! {
    TOOL_WINDOW_COMMANDS, scope = "RunToolWindow";
    /// Run the selected tab's process again.
    Rerun => "run.rerun", "Rerun",
        "Stops the selected Run tab's process if it runs and starts it again in the same tab.";
    /// Close a tab.
    CloseTab { index: Option<usize> } => "run.close-tab", "Close Tab",
        "Closes the selected Run tab, or the tab at a zero-based index; a running process is terminated after asking.",
        r#"{"index": 0}"#;
    /// Close the other tabs.
    CloseOtherTabs => "run.close-other-tabs", "Close Other Tabs",
        "Closes every Run tab but the selected one.";
    /// Close every tab.
    CloseAllTabs => "run.close-all-tabs", "Close All Tabs", "Closes every Run tab.";
    /// Select a tab.
    SelectTab { index: usize } => "run.select-tab", "Select Tab",
        "Selects the Run tab at a zero-based index.", r#"{"index": 0}"#;
    /// Pin or unpin a tab.
    TogglePin { index: Option<usize> } => "run.toggle-pin", "Pin Tab",
        "Pins the selected Run tab (a new run does not reuse a pinned tab), or unpins it.",
        r#"{"index": 0}"#;
    /// Go to the previous link of the console.
    PreviousLink => "run.console.previous-link", "Up the Stack Trace",
        "Goes to the previous link of the selected Run console and opens its target.";
    /// Go to the next link of the console.
    NextLink => "run.console.next-link", "Down the Stack Trace",
        "Goes to the next link of the selected Run console and opens its target.";
    /// Scroll the console to its end.
    ScrollToEnd => "run.console.scroll-to-end", "Scroll to End",
        "Scrolls the selected Run console to its end; it then follows new output.";
    /// Clear the console.
    Clear => "run.console.clear", "Clear All", "Clears the contents of the selected Run console.";
    /// Close the process's standard input.
    SendEof => "run.console.send-eof", "Send EOF",
        "Closes the standard input of the selected Run tab's process.";
    /// Type into the console.
    TypeInput { text: String } => "run.console.type", "Type into Console",
        "Types text into the selected Run console as the person would; a line break sends the line to the process.",
        r#"{"text": "42\n"}"#;
}
