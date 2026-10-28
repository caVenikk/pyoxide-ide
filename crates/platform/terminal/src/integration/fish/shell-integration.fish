# The IDE shell integration for fish (3.0 and later). the IDE starts fish with
# `--init-command="source <this file>"`, which runs after the user's
# configuration. The file:
# - sources the scripts the IDE names (a virtual environment's
#   `activate.fish`);
# - reports the working directory (OSC 7) and, before fish 4 (which marks
#   them itself), marks the prompt and the commands with their exit codes
#   (OSC 133 A, B, C, D) for the terminal.

# The scripts to source: __IDE_SOURCE_1, __IDE_SOURCE_2, ..., each
# "<script>\n<argument>\n...".
set -l __ide_i 1
while set -q __IDE_SOURCE_$__ide_i
    set -l __ide_name __IDE_SOURCE_$__ide_i
    set -l __ide_parts (string split \n -- $$__ide_name)
    set -e $__ide_name
    source $__ide_parts[1] $__ide_parts[2..-1]
    set __ide_i (math $__ide_i + 1)
end

# OSC 7: the working directory as a file URL, now and after every change.
function __ide_report_cwd --on-variable PWD
    printf '\e]7;file://%s%s\a' $hostname (string escape --style=url -- $PWD)
end
__ide_report_cwd

if test (string split . -- $version)[1] -lt 4
    function __ide_return
        return $argv[1]
    end

    if functions -q fish_prompt
        functions -c fish_prompt __ide_original_fish_prompt
    else
        function __ide_original_fish_prompt
            printf '> '
        end
    end

    function fish_prompt
        set -l last_status $status
        printf '\e]133;A\a'
        # The original prompt sees the status of the command.
        __ide_return $last_status
        __ide_original_fish_prompt
        printf '\e]133;B\a'
    end

    function __ide_preexec --on-event fish_preexec
        printf '\e]133;C\a'
    end

    function __ide_postexec --on-event fish_postexec
        printf '\e]133;D;%s\a' $status
    end
end
