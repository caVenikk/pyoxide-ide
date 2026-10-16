# Pyoxide shell integration for fish (3.0 and later). Pyoxide starts fish with
# `--init-command="source <this file>"`, which runs after the user's
# configuration. The file:
# - sources the scripts Pyoxide names (a virtual environment's
#   `activate.fish`);
# - reports the working directory (OSC 7) and, before fish 4 (which marks
#   them itself), marks the prompt and the commands with their exit codes
#   (OSC 133 A, B, C, D) for the terminal.

# The scripts to source: __PYOXIDE_SOURCE_1, __PYOXIDE_SOURCE_2, ..., each
# "<script>\n<argument>\n...".
set -l __pyoxide_i 1
while set -q __PYOXIDE_SOURCE_$__pyoxide_i
    set -l __pyoxide_name __PYOXIDE_SOURCE_$__pyoxide_i
    set -l __pyoxide_parts (string split \n -- $$__pyoxide_name)
    set -e $__pyoxide_name
    source $__pyoxide_parts[1] $__pyoxide_parts[2..-1]
    set __pyoxide_i (math $__pyoxide_i + 1)
end

# OSC 7: the working directory as a file URL, now and after every change.
function __pyoxide_report_cwd --on-variable PWD
    printf '\e]7;file://%s%s\a' $hostname (string escape --style=url -- $PWD)
end
__pyoxide_report_cwd

if test (string split . -- $version)[1] -lt 4
    function __pyoxide_return
        return $argv[1]
    end

    if functions -q fish_prompt
        functions -c fish_prompt __pyoxide_original_fish_prompt
    else
        function __pyoxide_original_fish_prompt
            printf '> '
        end
    end

    function fish_prompt
        set -l last_status $status
        printf '\e]133;A\a'
        # The original prompt sees the status of the command.
        __pyoxide_return $last_status
        __pyoxide_original_fish_prompt
        printf '\e]133;B\a'
    end

    function __pyoxide_preexec --on-event fish_preexec
        printf '\e]133;C\a'
    end

    function __pyoxide_postexec --on-event fish_postexec
        printf '\e]133;D;%s\a' $status
    end
end
