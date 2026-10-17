- crates/python/py-lsp/src/lookup.rs, settings.rs, idea.rs, servers.rs,
  ruff.rs, completion.rs
  Behaviour adapted from IntelliJ Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @ pycharm/2026.2.3:
  python/python-pytools/src/com/intellij/python/pytools/
  PyToolVersionCheck.kt (the version pattern
  `^<tool>,?(?:\s\(?version)?\s([^\s)]+).*$` and the message "The selected
  tool is not {0}, it is {1}"), PyToolExt.kt and configuration/
  ExecutableDiscoveryMode.kt (the lookup chain and the uvx arguments),
  PyToolsState.kt and the per-tool configuration classes of python/
  python-ruff, python-ty and python-pyright (the option names of
  .idea/pyLspTools.xml and their one-time migration), python/
  python-lsp-core/src/com/intellij/python/lsp/core/typeEngine/
  PyTypeEngineProjectSettings.kt (.idea/pyTypeEngine.xml), python/
  python-pyright/src/com/intellij/python/pyright/PyrightLspClient.kt (empty
  initialization options, an empty configuration change once running),
  python/python-ruff/src/com/intellij/python/ruff/server/RuffLspClient.kt
  (customizeQuickFixes: rule names in titles, Disable for this line nested)
  and codeinsight/actions/RuffDisableRuleIntentionAction.kt and
  RuffDisableRuleForFileIntentionAction.kt (the noqa comment pattern, the
  order of the configuration files, the layout of the ignore array). Only
  behaviour, formats, patterns and strings are used; reimplemented in Rust,
  no code is copied.
