# The IDE shell integration for zsh, sourced from .zshrc after the user's
# .zshrc (see .zshenv):
# - sources the scripts the IDE names (a virtual environment's `activate`)
#   before the first prompt;
# - reports the working directory (OSC 7) and marks the prompt and the
#   commands with their exit codes (OSC 133 A, B, C, D) for the terminal.

# The scripts to source: __IDE_SOURCE_1, __IDE_SOURCE_2, ..., each
# "<script>\n<argument>\n...". Sourced in a one-time precmd hook that runs
# before the other ones: after every startup file (.zlogin included), so
# nothing there undoes an activation, and before prompt themes look at it.
function __ide_source_scripts() {
  builtin emulate -L zsh
  builtin local __ide_i=1 __ide_name
  builtin local -a __ide_parts
  while true; do
    __ide_name="__IDE_SOURCE_${__ide_i}"
    [[ -n "${(P)__ide_name-}" ]] || break
    __ide_parts=("${(@f)${(P)__ide_name}}")
    builtin unset "$__ide_name"
    builtin source -- "${__ide_parts[1]}" "${(@)__ide_parts[2,-1]}"
    (( __ide_i++ ))
  done
}

function __ide_first_precmd() {
  builtin emulate -L zsh
  precmd_functions=(${precmd_functions:#__ide_first_precmd})
  __ide_source_scripts
  builtin unset -f __ide_source_scripts __ide_first_precmd
}

builtin typeset -ga precmd_functions
precmd_functions=(__ide_first_precmd $precmd_functions)

# The marks around the prompt (zero width for zsh).
__ide_prompt_start=$'%{\e]133;A\a%}'
__ide_prompt_end=$'%{\e]133;B\a%}'
# A command runs (between C and D).
__ide_command_running=

# OSC 7: the working directory as a file URL ('%' and control characters
# percent-encoded).
function __ide_report_cwd() {
  builtin emulate -L zsh
  builtin local dir="${PWD//\%/%25}"
  if [[ "$dir" == *[[:cntrl:]]* ]]; then
    builtin local LC_ALL=C out= c hex
    builtin local -i i
    for (( i = 1; i <= ${#dir}; i++ )); do
      c="${dir[i]}"
      if [[ "$c" == [[:cntrl:]] ]]; then
        builtin printf -v hex '%%%02X' "'$c"
        out+="$hex"
      else
        out+="$c"
      fi
    done
    dir="$out"
  fi
  builtin printf '\e]7;file://%s%s\a' "${HOST-}" "$dir"
}

function __ide_precmd() {
  builtin local status_code=$?
  builtin emulate -L zsh
  if [[ -n "$__ide_command_running" ]]; then
    __ide_command_running=
    builtin printf '\e]133;D;%s\a' "$status_code"
  fi
  __ide_report_cwd
  # The marks again each time: themes, `activate` and `deactivate` change PS1.
  PS1="${PS1//"$__ide_prompt_start"/}"
  PS1="${PS1//"$__ide_prompt_end"/}"
  PS1="${__ide_prompt_start}${PS1}${__ide_prompt_end}"
}

function __ide_preexec() {
  __ide_command_running=1
  builtin printf '\e]133;C\a'
}

builtin autoload -Uz add-zsh-hook
add-zsh-hook precmd __ide_precmd
add-zsh-hook preexec __ide_preexec

builtin unset __IDE_ZSH_DIR
