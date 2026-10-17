- crates/platform/debugger-ui (the breakpoints and their icons),
  crates/platform/debugger/src/provider.rs (the breakpoint types),
  crates/python/py-debug/src/breakpoints.rs (the Python types),
  crates/platform/workbench/keymaps/macos.toml and default.toml (breakpoint keys)
  Closely adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2): the icons of
  platform/icons/src/expui/breakpoints/ (breakpoint, breakpointDisabled,
  breakpointMuted, breakpointMutedDisabled, breakpointUnsuspendent,
  breakpointUnsuspendentValid, breakpointValid, breakpointInvalid,
  breakpointException, breakpointExceptionDisabled and questionBadge, light
  and dark; copied unchanged except questionBadge, set in a square box); the
  icon by state and the question badge's place of platform/xdebugger-impl/ui/
  src/com/intellij/xdebugger/impl/breakpoints/XBreakpointUIUtil.kt
  (calculateIcon); the tooltip of platform/xdebugger-impl/src/com/intellij/
  xdebugger/impl/breakpoints/XBreakpointBase.java (getDefaultDescription);
  the hover icon of platform/xdebugger-impl/frontend/src/com/intellij/
  platform/debugger/impl/frontend/editor/BreakpointPromoterEditorListener.java;
  the clicks of XLineBreakpointManager.kt, BreakpointGutterIconRenderer.java,
  RemoveBreakpointGutterIconAction.java, ToggleBreakpointGutterIconAction.java
  and actions/handlers/XToggleLineBreakpointActionHandler.kt; the line
  background rule of XBreakpointVisualRepresentation.kt; the properties popup
  of ui/DebuggerUIUtil.java (showXBreakpointEditorBalloon),
  ui/BreakpointEditor.java, breakpoints/ui/XLightBreakpointPropertiesPanel.java
  and XSuspendPolicyPanel.java; the Breakpoints dialog of
  breakpoints/ui/BreakpointsDialog.java and breakpoints/XBreakpointItem.java;
  the types of platform/xdebugger-api/src/com/intellij/xdebugger/breakpoints/
  XBreakpointType.java and XLineBreakpointType.java, of python/src/com/
  jetbrains/python/debugger/PyLineBreakpointType.java and
  PyExceptionBreakpointType.java (the activation policy); strings of
  XDebuggerBundle.properties and PyBundle.properties; the keys of
  ToggleLineBreakpoint, ToggleTemporaryLineBreakpoint, ViewBreakpoints and
  EditBreakpoint of platform/platform-resources/src/keymaps ($default.xml,
  Mac OS X 10.5+.xml). Changes: reimplemented in Rust with GPUI; the mute
  state is the project's (muted icons show while it is on); the class chooser
  is a list of the product's exception classes with speed search; the
  logging options, Remove once hit and the dependency are drawn disabled.
