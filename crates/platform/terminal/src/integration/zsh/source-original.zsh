# Pyoxide shell integration for zsh: sources the user's startup file named by
# __PYOXIDE_ORIGINAL_FILE in the global scope, with ZDOTDIR as the user has
# it while the file runs. Adapted from the IntelliJ Community terminal plugin
# (Apache-2.0), see NOTICE.

__pyoxide_original="${__PYOXIDE_ORIGINAL_ZDOTDIR:-$HOME}/${__PYOXIDE_ORIGINAL_FILE}"

if [[ -f "$__pyoxide_original" && "$ZDOTDIR" != "${__PYOXIDE_ORIGINAL_ZDOTDIR:-$HOME}" ]]; then
  __pyoxide_zdotdir="$ZDOTDIR"
  # The user's file may rely on ZDOTDIR.
  if [[ -n "${__PYOXIDE_ORIGINAL_ZDOTDIR-}" ]]; then
    ZDOTDIR="$__PYOXIDE_ORIGINAL_ZDOTDIR"
  else
    builtin unset ZDOTDIR
  fi

  builtin source "$__pyoxide_original"

  # The user's file may have changed ZDOTDIR: its later files are there.
  if [[ -n "${ZDOTDIR-}" ]]; then
    __PYOXIDE_ORIGINAL_ZDOTDIR="$ZDOTDIR"
  else
    builtin unset __PYOXIDE_ORIGINAL_ZDOTDIR
  fi
  ZDOTDIR="$__pyoxide_zdotdir"
fi

builtin unset __pyoxide_original __pyoxide_zdotdir __PYOXIDE_ORIGINAL_FILE
