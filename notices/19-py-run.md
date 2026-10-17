- crates/python/py-run
  Closely adapted from IntelliJ Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @
  82a18fd6f8095cced5d0cef964c041f63ece8ee5 (branch 262): the command line,
  environment and PYTHONPATH of python/src/com/jetbrains/python/run/
  PythonCommandLineState.java, PythonScripts.kt, TargetedPythonPaths.kt,
  PythonRunConfigurationProducer.java, python/python-sdk/src/com/jetbrains/
  python/sdk/PythonEnvUtil.java, PySdkUtil.java and python/src/com/jetbrains/
  python/sdk/PythonSdkType.java (virtual environment activation),
  python/src/com/jetbrains/python/sdk/uv/run/UvRunToolProvider.kt (uv run);
  the traceback filter of python/src/com/jetbrains/python/run/
  PythonTracebackFilter.kt and PyTracebackParser.kt (its regular
  expression), python/src/com/jetbrains/python/traceBackParsers/
  TraceBackParserAdapter.java, python/src/com/jetbrains/python/testing/
  pytest/PyTestTracebackParser.java and PyFilesStateMachine.java.
  Changes: reimplemented in Rust; the activation sets VIRTUAL_ENV and PATH
  directly instead of reading the environment of the activate script.
