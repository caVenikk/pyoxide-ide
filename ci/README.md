# ci/

GitHub rejects tool-written files under `.github/`, so the templates and the
workflow are staged in `ci/github/`. Move them once:

    mv ci/github .github && rmdir ci 2>/dev/null; rm -f ci/README.md

(or `git mv` after the first commit). Then delete this folder.
