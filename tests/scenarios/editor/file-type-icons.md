# File type icons

## Steps
1. Open a folder holding, at its root: `.dockerignore`, `.env`,
   `.env.example`, `.gitignore`, `.gitlab-ci.yml`,
   `.pre-commit-config.yaml`, `.python-version` (`3.13`), `alembic.ini`,
   `CLAUDE.md`, `docker-compose.yml`, `docker-compose.dev.yml`,
   `Dockerfile`, `Dockerfile.dev` (`FROM python`), `logs.log`,
   `pyproject.toml`, `README.md`, `run.py`, `uv.lock`, `LICENSE` (text),
   `data.bin` (bytes 0 and 1) and `manage` (`#!/usr/bin/env python3`).
2. Open `.gitlab-ci.yml`, `manage` and `.env.example` in the editor.
3. Overwrite `data.bin` with text and choose File > Reload All from Disk.

## Expected
- Step 1: `.dockerignore` shows a grey whale; `docker-compose*.yml` a blue
  whale carrying three containers; `Dockerfile` the blue whale with one
  container; `.gitlab-ci.yml` the GitLab logo; `.pre-commit-config.yaml` the
  YAML icon (a red file with a "Y" inside), like every YAML file; `.gitignore` a crossed-out circle; `alembic.ini`
  the INI icon; `pyproject.toml` and `uv.lock` the TOML "T"; the `.md`
  files the Markdown "M↓"; `run.py` and `manage` the Python icon. `.env`,
  `.env.example`, `.python-version`, `Dockerfile.dev`, `logs.log` and
  `LICENSE` show the text icon (lines); `data.bin` the unknown-file "?".
- Step 2: the tabs show the same icons as the tree; `manage` is
  highlighted as Python; Comment with Line Comment in `.env.example` and
  `.gitlab-ci.yml` inserts `# `.
- Step 3: `data.bin` now shows the text icon.

The type of a file comes from its whole name (case-sensitive), then its
whole name in any case (`REQUIREMENTS.TXT`), then name patterns (the most
specific first: longer, then other characters before `?` before `*`),
then its extension in any case; a file none of them names is typed by its
first bytes: a `#!` line naming `python` or `bash`/`sh`/`zsh`, text (an
empty file too) or binary (a byte below 9).

## Commands
`project.open-folder {"path"}`, `project.open-file {"path"}`,
`workbench.synchronize`, `workbench.state` (`tree.rows[].icon`).

## Source
IntelliJ Platform Community at `pycharm/2026.2.3` (Apache-2.0):
`FileTypeAssocTable`, `FileNameMatcherFactoryImpl`,
`WildcardFileNameMatcher`, `FileTypeManagerImpl`,
`FileTypeDetectionService`, `DetectedByContentFileType`, `CharsetToolkit`
(`guessFromContent`), `FileUtil.isHashBangLine`; the file types registered
in the plugin descriptors of the platform, `images`, `json`, `jupyter`,
`plugins/{yaml,toml,markdown,properties,restructuredtext,sh,editorconfig,
env-files-support,git4idea,hg4idea}`, `platform/vcs-impl`,
`xml/xml-psi-impl`, `python/{python-parser,python-pyproject,conda}` and
`python/pluginResources`; `GitlabCiIconProvider`,
`GithubYamlIconProvider` / `githubYamlFileDetection.kt`. Observed in
PyCharm 2026.2.3 on a project root: the Docker, Docker Compose,
`.dockerignore` and INI icons (closed-source plugins; Pyoxide
draws its own; the YAML icon is drawn anew too), pre-commit files with the
plain YAML icon, `Dockerfile.*` drawn as text.
