# The IDE shell integration for zsh: sources the user's startup file named by
# __IDE_ORIGINAL_FILE in the global scope, with ZDOTDIR as the user has
# it while the file runs. Adapted from the IntelliJ Community terminal plugin
# (Apache-2.0), see NOTICE.

__ide_original="${__IDE_ORIGINAL_ZDOTDIR:-$HOME}/${__IDE_ORIGINAL_FILE}"

if [[ -f "$__ide_original" && "$ZDOTDIR" != "${__IDE_ORIGINAL_ZDOTDIR:-$HOME}" ]]; then
  __ide_zdotdir="$ZDOTDIR"
  # The user's file may rely on ZDOTDIR.
  if [[ -n "${__IDE_ORIGINAL_ZDOTDIR-}" ]]; then
    ZDOTDIR="$__IDE_ORIGINAL_ZDOTDIR"
  else
    builtin unset ZDOTDIR
  fi

  builtin source "$__ide_original"

  # The user's file may have changed ZDOTDIR: its later files are there.
  if [[ -n "${ZDOTDIR-}" ]]; then
    __IDE_ORIGINAL_ZDOTDIR="$ZDOTDIR"
  else
    builtin unset __IDE_ORIGINAL_ZDOTDIR
  fi
  ZDOTDIR="$__ide_zdotdir"
fi

builtin unset __ide_original __ide_zdotdir __IDE_ORIGINAL_FILE
