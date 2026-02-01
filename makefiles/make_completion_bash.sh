#!/bin/bash
# scripts/make_completion.bash - Bash completion for gvthread Makefile
#
# Installation:
#   source scripts/make_completion.bash
#
# Or add to ~/.bashrc:
#   source /path/to/gvthread/scripts/make_completion.bash

_gvthread_make_completion() {
    local cur="${COMP_WORDS[COMP_CWORD]}"
    local makefile="Makefile"
    
    # Check if we're in the gvthread directory
    if [[ ! -f "$makefile" ]]; then
        return
    fi
    
    # Get targets from Python parser
    local targets
    targets=$(MPAT=1 python3 scripts/makehelper.py \
        Makefile makefiles/*.mk 2>/dev/null)
    
    COMPREPLY=($(compgen -W "$targets" -- "$cur"))
}

# Register completion for 'make' when in gvthread directory
complete -F _gvthread_make_completion make

# Also register for common aliases
complete -F _gvthread_make_completion m