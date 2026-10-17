- crates/platform/text/src/reload.rs
  Closely adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3
  (404a109fe630a2b9013f10b71081f87c9dbb32e2). Files:
  platform/core-impl/src/com/intellij/openapi/editor/impl/event/DocumentEventImpl.java
  (translateLineViaDiff), platform/core-impl/src/com/intellij/openapi/editor/impl/DocumentImpl.java
  (replaceString: the common start and end of a whole-text replacement are
  kept), platform/platform-impl/src/com/intellij/openapi/editor/impl/CaretImpl.java
  (a caret inside the replaced part keeps its column on the translated
  line, else stays within the new part). Changes: reimplemented in Rust on
  a rope with char offsets; the line diff is Pyoxide's own (Myers' algorithm
  with a cost limit).
