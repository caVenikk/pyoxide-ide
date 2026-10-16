# Pyoxide shell integration for bash (3.2 and later). Pyoxide starts bash as
# `bash --rcfile <this file> -i`; for a login shell it leaves out --login,
# which would make bash skip --rcfile, and sets __PYOXIDE_LOGIN_SHELL. (sh,
# bash in POSIX mode, reads pyoxide-integration.sh first, which reads its
# startup files and then this one.) The file reads the user's startup files
# as bash would, then:
# - sources the scripts Pyoxide names (a virtual environment's `activate`);
# - reports the working directory (OSC 7) and marks the prompt and the
#   commands with their exit codes (OSC 133 A, B, C, D) for the terminal.
# The startup part is adapted from the IntelliJ Community terminal plugin
# (Apache-2.0), see NOTICE.

if [ -n "${__PYOXIDE_SH-}" ]; then
  # From pyoxide-integration.sh: the startup files are read.
  builtin unset __PYOXIDE_SH
elif [ -n "${__PYOXIDE_LOGIN_SHELL-}" ]; then
  builtin unset __PYOXIDE_LOGIN_SHELL
  # What an interactive login shell reads.
  if [ -f /etc/profile ]; then
    builtin source /etc/profile
  fi
  if [ -f ~/.bash_profile ]; then
    builtin source ~/.bash_profile
  elif [ -f ~/.bash_login ]; then
    builtin source ~/.bash_login
  elif [ -f ~/.profile ]; then
    builtin source ~/.profile
  fi
elif [ -n "${__PYOXIDE_USER_RCFILE-}" ]; then
  # The user started bash with --rcfile of their own.
  builtin source "$__PYOXIDE_USER_RCFILE"
elif [ -f ~/.bashrc ]; then
  builtin source ~/.bashrc
fi
builtin unset __PYOXIDE_USER_RCFILE

# The scripts to source: __PYOXIDE_SOURCE_1, __PYOXIDE_SOURCE_2, ..., each
# "<script>\n<argument>\n...", in the global scope after the startup files.
__pyoxide_i=1
while true; do
  __pyoxide_name="__PYOXIDE_SOURCE_${__pyoxide_i}"
  __pyoxide_value="${!__pyoxide_name-}"
  [ -n "$__pyoxide_value" ] || break
  builtin unset "$__pyoxide_name"
  __pyoxide_parts=()
  while IFS= builtin read -r __pyoxide_line; do
    __pyoxide_parts[${#__pyoxide_parts[@]}]="$__pyoxide_line"
  done <<< "$__pyoxide_value"
  builtin source "${__pyoxide_parts[0]}" "${__pyoxide_parts[@]:1}"
  __pyoxide_i=$((__pyoxide_i + 1))
done
builtin unset __pyoxide_i __pyoxide_name __pyoxide_value __pyoxide_parts __pyoxide_line

# The marks around the prompt (non-printing for bash).
__pyoxide_prompt_start='\[\033]133;A\007\]'
__pyoxide_prompt_end='\[\033]133;B\007\]'
# The status of the last command, kept over PROMPT_COMMAND.
__pyoxide_status=0
# The prompt was shown: the next command the DEBUG trap sees is the user's.
__pyoxide_ready=
# A command runs (between C and D).
__pyoxide_running=

# OSC 7: the working directory as a file URL ('%' and control characters
# percent-encoded; Git Bash and Cygwin report the Windows path).
__pyoxide_report_cwd() {
  builtin local dir="$PWD"
  case "${OSTYPE-}" in
    msys* | cygwin*)
      dir="/$(builtin pwd -W 2>/dev/null || builtin command cygpath -m "$PWD" 2>/dev/null || builtin printf '%s' "$PWD")"
      ;;
  esac
  dir="${dir//\%/%25}"
  if [[ "$dir" == *[[:cntrl:]]* ]]; then
    builtin local LC_ALL=C out= c hex i
    for ((i = 0; i < ${#dir}; i++)); do
      c="${dir:i:1}"
      case "$c" in
        [[:cntrl:]])
          builtin printf -v hex '%%%02X' "'$c"
          out="$out$hex"
          ;;
        *) out="$out$c" ;;
      esac
    done
    dir="$out"
  fi
  builtin printf '\033]7;file://%s%s\007' "${HOSTNAME-}" "$dir"
}

# The first entry of PROMPT_COMMAND.
__pyoxide_precmd_start() {
  __pyoxide_status=$?
  __pyoxide_ready=
  return "$__pyoxide_status"
}

# The last entry of PROMPT_COMMAND.
__pyoxide_precmd_end() {
  if [ -n "$__pyoxide_running" ]; then
    __pyoxide_running=
    builtin printf '\033]133;D;%s\007' "$__pyoxide_status"
  fi
  __pyoxide_report_cwd
  # The marks again each time: themes, `activate` and `deactivate` change PS1.
  PS1="${PS1//"$__pyoxide_prompt_start"/}"
  PS1="${PS1//"$__pyoxide_prompt_end"/}"
  PS1="$__pyoxide_prompt_start$PS1$__pyoxide_prompt_end"
  __pyoxide_ready=1
  # The prompt may show the status of the command.
  return "$__pyoxide_status"
}

# Whether the command the DEBUG trap sees belongs to PROMPT_COMMAND: Enter on
# an empty line or Ctrl+C at the prompt runs it without a user command.
__pyoxide_is_prompt_command() {
  case "$1" in
    __pyoxide_precmd_start* | __pyoxide_precmd_end*) return 0 ;;
  esac
  builtin local nl='
'
  case "$nl${PROMPT_COMMAND[*]-}$nl" in
    *"$nl$1$nl"*) return 0 ;;
  esac
  return 1
}

# The DEBUG trap: the first command after the prompt is the user's (C). A
# DEBUG trap set before is run after it; "$_" is passed through to it.
__pyoxide_debug_trap() {
  if [ -n "$__pyoxide_ready" ]; then
    case "${FUNCNAME[1]-}" in
      __pyoxide_*) ;;
      *)
        __pyoxide_ready=
        if ! __pyoxide_is_prompt_command "${BASH_COMMAND-}"; then
          __pyoxide_running=1
          builtin printf '\033]133;C\007'
        fi
        ;;
    esac
  fi
  if [ -n "${__pyoxide_original_debug_trap-}" ]; then
    : "$1"
    builtin eval "$__pyoxide_original_debug_trap"
  fi
}

# The code of a DEBUG trap set before (`trap -p` prints `trap -- 'code' DEBUG`).
__pyoxide_original_debug_trap="$(
  builtin eval "__pyoxide_trap=($(builtin trap -p DEBUG))"
  builtin printf '%s' "${__pyoxide_trap[2]-}"
)"
builtin trap '__pyoxide_debug_trap "$_"' DEBUG

if [[ "$(builtin declare -p PROMPT_COMMAND 2>/dev/null)" == "declare -a"* ]]; then
  PROMPT_COMMAND=(__pyoxide_precmd_start "${PROMPT_COMMAND[@]}" __pyoxide_precmd_end)
else
  __pyoxide_nl='
'
  PROMPT_COMMAND="__pyoxide_precmd_start${__pyoxide_nl}${PROMPT_COMMAND:+$PROMPT_COMMAND$__pyoxide_nl}__pyoxide_precmd_end"
  builtin unset __pyoxide_nl
fi
