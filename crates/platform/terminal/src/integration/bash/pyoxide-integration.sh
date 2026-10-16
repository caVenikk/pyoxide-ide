# Pyoxide shell integration for sh (POSIX syntax only). /bin/sh of macOS is
# bash in POSIX mode, which ignores --rcfile: an interactive one reads the file
# named by $ENV, after /etc/profile and ~/.profile when it is a login shell.
# Pyoxide starts it without --login (and sets __PYOXIDE_LOGIN_SHELL) and with
# ENV naming this file, which:
# - puts back the user's ENV (__PYOXIDE_USER_ENV) and reads the user's
#   startup files as sh would: the profile files, then the file $ENV names;
# - in bash, goes on with pyoxide-integration.bash next to it (the scripts
#   Pyoxide names to source and the marks);
# - in another shell selected as /bin/sh (dash, zsh), sources the scripts
#   Pyoxide names (a virtual environment's `activate`), without the marks.

__pyoxide_sh_dir="${ENV%/*}"
if [ -n "${__PYOXIDE_USER_ENV+set}" ]; then
  ENV="$__PYOXIDE_USER_ENV"
  unset __PYOXIDE_USER_ENV
else
  unset ENV
fi

if [ -n "${__PYOXIDE_LOGIN_SHELL-}" ]; then
  unset __PYOXIDE_LOGIN_SHELL
  if [ -f /etc/profile ]; then
    . /etc/profile
  fi
  if [ -f "${HOME-}/.profile" ]; then
    . "$HOME/.profile"
  fi
fi
# The user's ENV, expanded as sh expands it ($HOME/.shrc).
if [ -n "${ENV-}" ]; then
  eval "__pyoxide_sh_env=\"$ENV\""
  if [ -f "$__pyoxide_sh_env" ]; then
    . "$__pyoxide_sh_env"
  fi
  unset __pyoxide_sh_env
fi

if [ -n "${BASH_VERSION-}" ]; then
  __pyoxide_sh_bash="$__pyoxide_sh_dir/pyoxide-integration.bash"
  unset __pyoxide_sh_dir
  __PYOXIDE_SH=1
  . "$__pyoxide_sh_bash"
else
  unset __pyoxide_sh_dir
  # The scripts to source: __PYOXIDE_SOURCE_1, __PYOXIDE_SOURCE_2, ..., each
  # "<script>\n<argument>\n...". A function, so that `set --` gives the
  # script its arguments; what the script sets stays global.
  __pyoxide_sh_source() {
    __pyoxide_sh_value="$1"
    __pyoxide_sh_script=
    __pyoxide_sh_first=1
    set --
    while IFS= read -r __pyoxide_sh_line; do
      if [ -n "$__pyoxide_sh_first" ]; then
        __pyoxide_sh_script="$__pyoxide_sh_line"
        __pyoxide_sh_first=
      else
        set -- "$@" "$__pyoxide_sh_line"
      fi
    done <<__PYOXIDE_EOF
$__pyoxide_sh_value
__PYOXIDE_EOF
    unset __pyoxide_sh_value __pyoxide_sh_first __pyoxide_sh_line
    . "$__pyoxide_sh_script"
  }
  __pyoxide_sh_i=1
  while :; do
    eval "__pyoxide_sh_entry=\"\${__PYOXIDE_SOURCE_${__pyoxide_sh_i}-}\""
    [ -n "$__pyoxide_sh_entry" ] || break
    unset "__PYOXIDE_SOURCE_${__pyoxide_sh_i}"
    __pyoxide_sh_source "$__pyoxide_sh_entry"
    __pyoxide_sh_i=$((__pyoxide_sh_i + 1))
  done
  unset -f __pyoxide_sh_source
  unset __pyoxide_sh_i __pyoxide_sh_entry __pyoxide_sh_script
fi
unset __pyoxide_sh_bash
