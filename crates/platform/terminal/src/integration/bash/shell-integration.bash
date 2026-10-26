# The IDE shell integration for bash (3.2 and later). the IDE starts bash as
# `bash --rcfile <this file> -i`; for a login shell it leaves out --login,
# which would make bash skip --rcfile, and sets __IDE_LOGIN_SHELL. (sh,
# bash in POSIX mode, reads shell-integration.sh first, which reads its
# startup files and then this one.) The file reads the user's startup files
# as bash would, then:
# - sources the scripts the IDE names (a virtual environment's `activate`);
# - reports the working directory (OSC 7) and marks the prompt and the
#   commands with their exit codes (OSC 133 A, B, C, D) for the terminal.
# The startup part is adapted from the IntelliJ Community terminal plugin
# (Apache-2.0), see NOTICE.

if [ -n "${__IDE_SH-}" ]; then
  # From shell-integration.sh: the startup files are read.
  builtin unset __IDE_SH
elif [ -n "${__IDE_LOGIN_SHELL-}" ]; then
  builtin unset __IDE_LOGIN_SHELL
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
elif [ -n "${__IDE_USER_RCFILE-}" ]; then
  # The user started bash with --rcfile of their own.
  builtin source "$__IDE_USER_RCFILE"
elif [ -f ~/.bashrc ]; then
  builtin source ~/.bashrc
fi
builtin unset __IDE_USER_RCFILE

# The scripts to source: __IDE_SOURCE_1, __IDE_SOURCE_2, ..., each
# "<script>\n<argument>\n...", in the global scope after the startup files.
__ide_i=1
while true; do
  __ide_name="__IDE_SOURCE_${__ide_i}"
  __ide_value="${!__ide_name-}"
  [ -n "$__ide_value" ] || break
  builtin unset "$__ide_name"
  __ide_parts=()
  while IFS= builtin read -r __ide_line; do
    __ide_parts[${#__ide_parts[@]}]="$__ide_line"
  done <<< "$__ide_value"
  builtin source "${__ide_parts[0]}" "${__ide_parts[@]:1}"
  __ide_i=$((__ide_i + 1))
done
builtin unset __ide_i __ide_name __ide_value __ide_parts __ide_line

# The marks around the prompt (non-printing for bash).
__ide_prompt_start='\[\033]133;A\007\]'
__ide_prompt_end='\[\033]133;B\007\]'
# The status of the last command, kept over PROMPT_COMMAND.
__ide_status=0
# The prompt was shown: the next command the DEBUG trap sees is the user's.
__ide_ready=
# A command runs (between C and D).
__ide_running=

# OSC 7: the working directory as a file URL ('%' and control characters
# percent-encoded; Git Bash and Cygwin report the Windows path).
__ide_report_cwd() {
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
__ide_precmd_start() {
  __ide_status=$?
  __ide_ready=
  return "$__ide_status"
}

# The last entry of PROMPT_COMMAND.
__ide_precmd_end() {
  if [ -n "$__ide_running" ]; then
    __ide_running=
    builtin printf '\033]133;D;%s\007' "$__ide_status"
  fi
  __ide_report_cwd
  # The marks again each time: themes, `activate` and `deactivate` change PS1.
  PS1="${PS1//"$__ide_prompt_start"/}"
  PS1="${PS1//"$__ide_prompt_end"/}"
  PS1="$__ide_prompt_start$PS1$__ide_prompt_end"
  __ide_ready=1
  # The prompt may show the status of the command.
  return "$__ide_status"
}

# Whether the command the DEBUG trap sees belongs to PROMPT_COMMAND: Enter on
# an empty line or Ctrl+C at the prompt runs it without a user command.
__ide_is_prompt_command() {
  case "$1" in
    __ide_precmd_start* | __ide_precmd_end*) return 0 ;;
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
__ide_debug_trap() {
  if [ -n "$__ide_ready" ]; then
    case "${FUNCNAME[1]-}" in
      __ide_*) ;;
      *)
        __ide_ready=
        if ! __ide_is_prompt_command "${BASH_COMMAND-}"; then
          __ide_running=1
          builtin printf '\033]133;C\007'
        fi
        ;;
    esac
  fi
  if [ -n "${__ide_original_debug_trap-}" ]; then
    : "$1"
    builtin eval "$__ide_original_debug_trap"
  fi
}

# The code of a DEBUG trap set before (`trap -p` prints `trap -- 'code' DEBUG`).
__ide_original_debug_trap="$(
  builtin eval "__ide_trap=($(builtin trap -p DEBUG))"
  builtin printf '%s' "${__ide_trap[2]-}"
)"
builtin trap '__ide_debug_trap "$_"' DEBUG

if [[ "$(builtin declare -p PROMPT_COMMAND 2>/dev/null)" == "declare -a"* ]]; then
  PROMPT_COMMAND=(__ide_precmd_start "${PROMPT_COMMAND[@]}" __ide_precmd_end)
else
  __ide_nl='
'
  PROMPT_COMMAND="__ide_precmd_start${__ide_nl}${PROMPT_COMMAND:+$PROMPT_COMMAND$__ide_nl}__ide_precmd_end"
  builtin unset __ide_nl
fi
