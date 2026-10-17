- crates/platform/editor/src/wrap.rs, soft_wrap.rs, element.rs (soft
  wraps), crates/platform/workbench/src/editor_settings.rs,
  crates/python/pyoxide-ide/src/lib.rs (Python wrap rules)
  Closely adapted from the IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3: the
  wrap positions of platform/platform-impl/src/com/intellij/openapi/editor/
  impl/SoftWrapEngine.java (rows filled up to the view's width with room
  for the wrap sign, the ten-char look-back for a space, the continuation
  indent of the sign plus the line's indentation),
  platform/editor-ui-api/src/com/intellij/openapi/editor/
  LineWrapPositionStrategy.java (where a line may wrap) and
  platform/code-style-api/src/com/intellij/openapi/editor/
  GenericLineWrapPositionStrategy.java with DefaultLineWrapPositionStrategy
  .java (the rules, their weights and the choice between places); the
  width of .../impl/softwrap/mapping/SoftWrapApplianceManager.java (no
  wrapping under the scrollbar); Home and End on soft-wrapped lines of
  platform/platform-impl/src/com/intellij/openapi/editor/actions/
  EditorActionUtil.java; the anchoring of the view of
  AbstractToggleUseSoftWrapsAction.java; the "Soft-Wrap" action and its
  place of platform/platform-resources/src/idea/PlatformActions.xml and
  platform/platform-resources-en/src/messages/ActionsBundle.properties; the
  defaults of platform/ide-core-impl/src/com/intellij/openapi/editor/ex/
  EditorSettingsExternalizable.java; the signs of .../impl/softwrap/
  CompositeSoftWrapPainter.java and where .../impl/view/EditorPainter.java
  paints them (the caret's line only unless all soft wraps are shown); the
  Python rules of python/python-syntax-core/src/com/jetbrains/python/
  formatter/PyLineWrapPositionStrategy.java. Changes: reimplemented in Rust over
  display columns of the monospace editor font instead of pixels, one char
  per element (no fold regions, inlays or custom wraps), the rows of all
  lines kept as counts with prefix sums and computed off the UI thread for
  large texts; no caret position inside a continuation row's indent.
