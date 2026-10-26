# The IDE shell integration for sh (POSIX syntax only). /bin/sh of macOS is
# bash in POSIX mode, which ignores --rcfile: an interactive one reads the file
# named by $ENV, after /etc/profile and ~/.profile when it is a login shell.
# The IDE starts it without --login (and sets __IDE_LOGIN_SHELL) and with
# ENV naming this file, which:
# - puts back the user's ENV (__IDE_USER_ENV) and reads the user's
#   startup files as sh would: the profile files, then the file $ENV names;
# - in bash, goes on with shell-integration.bash next to it (the scripts
#   the IDE names to source and the marks);
# - in another shell selected as /bin/sh (dash, zsh), sources the scripts
#   the IDE names (a virtual environment's `activate`), without the marks.

__ide_sh_dir="${ENV%/*}"
if [ -n "${__IDE_USER_ENV+set}" ]; then
  ENV="$__IDE_USER_ENV"
  unset __IDE_USER_ENV
else
  unset ENV
fi

if [ -n "${__IDE_LOGIN_SHELL-}" ]; then
  unset __IDE_LOGIN_SHELL
  if [ -f /etc/profile ]; then
    . /etc/profile
  fi
  if [ -f "${HOME-}/.profile" ]; then
    . "$HOME/.profile"
  fi
fi
# The user's ENV, expanded as sh expands it ($HOME/.shrc).
if [ -n "${ENV-}" ]; then
  eval "__ide_sh_env=\"$ENV\""
  if [ -f "$__ide_sh_env" ]; then
    . "$__ide_sh_env"
  fi
  unset __ide_sh_env
fi

if [ -n "${BASH_VERSION-}" ]; then
  __ide_sh_bash="$__ide_sh_dir/shell-integration.bash"
  unset __ide_sh_dir
  __IDE_SH=1
  . "$__ide_sh_bash"
else
  unset __ide_sh_dir
  # The scripts to source: __IDE_SOURCE_1, __IDE_SOURCE_2, ..., each
  # "<script>\n<argument>\n...". A function, so that `set --` gives the
  # script its arguments; what the script sets stays global.
  __ide_sh_source() {
    __ide_sh_value="$1"
    __ide_sh_script=
    __ide_sh_first=1
    set --
    while IFS= read -r __ide_sh_line; do
      if [ -n "$__ide_sh_first" ]; then
        __ide_sh_script="$__ide_sh_line"
        __ide_sh_first=
      else
        set -- "$@" "$__ide_sh_line"
      fi
    done <<__IDE_EOF
$__ide_sh_value
__IDE_EOF
    unset __ide_sh_value __ide_sh_first __ide_sh_line
    . "$__ide_sh_script"
  }
  __ide_sh_i=1
  while :; do
    eval "__ide_sh_entry=\"\${__IDE_SOURCE_${__ide_sh_i}-}\""
    [ -n "$__ide_sh_entry" ] || break
    unset "__IDE_SOURCE_${__ide_sh_i}"
    __ide_sh_source "$__ide_sh_entry"
    __ide_sh_i=$((__ide_sh_i + 1))
  done
  unset -f __ide_sh_source
  unset __ide_sh_i __ide_sh_entry __ide_sh_script
fi
unset __ide_sh_bash
