- crates/platform/console
  Closely adapted from IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @
  82a18fd6f8095cced5d0cef964c041f63ece8ee5 (branch 262): the escape
  sequence lexer of platform/platform-util-io/src/com/intellij/execution/
  process/AnsiStreamingLexer.java, the SGR codes and the 256-colour table
  of AnsiTerminalEmulator.java, AnsiCommands.java, AnsiEscapeDecoder.java
  and platform/platform-api/src/com/intellij/execution/process/
  ColoredOutputTypeRegistryImpl.java; the carriage return and backspace
  rules of platform/execution-impl/src/com/intellij/execution/impl/
  TokenBuffer.java, platform/lang-impl/src/com/intellij/execution/impl/
  ConsoleViewImpl.kt and ConsoleTokenUtil.java and of platform/util/src/
  com/intellij/util/io/BaseOutputReader.java; the cycle buffer of
  .../execution/impl/ConsoleBuffer.java and platform/core-impl/src/com/
  intellij/openapi/editor/impl/DocumentImpl.java; the URL filter and its
  regular expressions of platform/execution-impl/src/com/intellij/execution/
  filters/UrlFilter.java and platform/util/src/com/intellij/util/io/
  URLUtil.java (URL_PATTERN_OPTIMIZED, FILE_URL_PATTERN_OPTIMIZED).
  Changes: reimplemented in Rust on the `regex` crate, whose syntax has no
  look-behind (checked by hand); text is UTF-8 with byte ranges.
