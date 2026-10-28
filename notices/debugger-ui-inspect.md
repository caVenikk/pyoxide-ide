- crates/platform/debugger-ui/src/inspect.rs (Inspect dialog, Compare Value with
  Clipboard window, Show Referring Objects), crates/platform/debugger-ui/src/variables.rs
  (Protected Attributes group), crates/platform/debugger-ui/assets/icons/values/accessProtected*.svg
  and unmarkWebRoot*.svg, crates/python/py-debug/src/values.rs and provider.rs
  (the Python grouping, `__len__`, the referrers expression)
  Closely adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): the icons
  platform/icons/src/expui/nodes/accessProtected.svg and unmarkWebRoot.svg (light
  and dark), copied unchanged; the Inspect dialog of
  platform/xdebugger-impl/ui/src/com/intellij/xdebugger/impl/actions/XInspectAction.java
  and ui/tree/XInspectDialog.java and DebuggerTreeWithHistoryPanel.java (the
  toolbar with Set As Root, Back and Forward, a history of 11 steps, the tree
  rebuilt when the session pauses); actions/XCompareWithClipboardAction.java and
  the diff request of DiffRequestFactoryImpl.java (clipboard titled "From
  clipboard", value titled "Selected Value", window title "Clipboard vs
  Selected Value"); the referring objects of
  python/pydevSrc/src/com/jetbrains/python/debugger/PyReferringObjectsValue.java
  and the dialog title of XDebuggerBundle.properties; the grouping of
  python/pydevSrc/src/com/jetbrains/python/debugger/PyDebugValue.java
  (computeChildren: names starting with an underscore, except `__len__`, in a
  "Protected Attributes" group) and PyDebugValueGroups.kt; strings of
  XDebuggerBundle.properties and DiffBundle.properties. Changes: reimplemented
  in Rust with GPUI; debugpy has no referrer request, so the referrers are
  listed by evaluating `gc.get_referrers` in the frame; the sequence length
  `__len__` is a row of the container's children, evaluated like its size.
