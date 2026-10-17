- crates/platform/workbench/src/file_types.rs, crates/python/pyoxide-ide/
  src/lib.rs (file types), crates/platform/project/src/filter.rs
  (wildcard_match)
  File types follow the IntelliJ Platform Community (Apache-2.0),
  https://github.com/JetBrains/intellij-community @
  404a109fe630a2b9013f10b71081f87c9dbb32e2 (tag pycharm/2026.2.3): the
  lookup order and pattern order of jps/model-impl/src/com/intellij/
  openapi/fileTypes/impl/FileTypeAssocTable.java, the masks of jps/
  model-impl/src/org/jetbrains/jps/model/fileTypes/impl/
  FileNameMatcherFactoryImpl.java and jps/model-impl/src/com/intellij/
  openapi/fileTypes/WildcardFileNameMatcher.java, the detection from
  content of platform/platform-impl/src/com/intellij/openapi/fileTypes/
  impl/FileTypeManagerImpl.java, FileTypeDetectionService.java and
  DetectedByContentFileType.java, platform/util/src/com/intellij/openapi/
  vfs/CharsetToolkit.java (guessFromContent) and platform/util/src/com/
  intellij/openapi/util/io/FileUtil.java (isHashBangLine); the names,
  extensions, patterns, `#!` words and icons of the file types registered
  in images/resources/META-INF/plugin.xml, json/resources/
  intellij.json.xml, jupyter/resources/intellij.notebooks.jupyter.core.xml,
  plugins/{yaml/resources/intellij.yaml.xml, toml/core/src/main/resources/
  intellij.toml.core.xml, markdown/core/resources/META-INF/plugin.xml,
  properties/properties-common/resources/intellij.properties.xml,
  restructuredtext/resources/META-INF/plugin.xml, sh/core/resources/
  intellij.sh.core.xml, editorconfig/common/resources/
  intellij.editorconfig.common.xml, env-files-support/src/main/resources/
  META-INF/plugin.xml, git4idea/backend/resources/
  intellij.vcs.git.backend.xml, hg4idea/resources/META-INF/plugin.xml},
  platform/vcs-impl/resources/intellij.platform.vcs.impl.xml,
  xml/xml-psi-impl/resources/intellij.xml.psi.impl.xml, python/
  python-parser/resources/intellij.python.parser.xml, python/
  python-pyproject/resources/intellij.python.pyproject.xml, python/conda/
  resources/intellij.python.community.impl.conda.xml and python/
  pluginResources/intellij.python.community.impl.xml, with the icons of
  their classes (DotEnvFileType, RequirementsIcons, PyTypedFileType,
  SvgFileType, JupyterFileType); the icon rules of plugins/gitlab/
  gitlab-core/src/org/jetbrains/plugins/gitlab/ui/GitlabCiIconProvider.kt
  and plugins/github/github-core/src/org/jetbrains/plugins/github/
  extensions/GithubYamlIconProvider.kt and githubYamlFileDetection.kt.
  Only behaviour and data are used; reimplemented in Rust, no code is
  copied.
