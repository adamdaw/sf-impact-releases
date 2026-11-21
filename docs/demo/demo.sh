#!/usr/bin/env bash
# The terminal demo that docs/demo/demo.svg plays.
#
# Builds nothing: it runs the five released commands from PATH on a throwaway
# copy of fixtures/project, made into a git repository with a base commit on
# `main` and one commit on a branch that changes an Apex class. The commands
# are typed out for the recording, then run for real, so every line of output
# is what the tools wrote. regenerate.sh runs this inside a container with the
# project mounted at /fixtures/project and the work tree at /work.
set -eu

fixtures=${SF_IMPACT_FIXTURES:-/fixtures/project}
work=${SF_IMPACT_WORK:-/work}

# Fixed identity and dates, so the commit ids on screen are the same every run.
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 GIT_PAGER=cat
export GIT_AUTHOR_NAME=Demo GIT_AUTHOR_EMAIL=demo@example.com
export GIT_COMMITTER_NAME=Demo GIT_COMMITTER_EMAIL=demo@example.com

setup() {
    rm -rf "$work"
    cp -R "$fixtures" "$work"
    cd "$work"
    # The fixture declares a package directory that does not exist, to test
    # the warning for it. The demo leaves that one entry out.
    sed -i '/"not-there"/d; s/" },$/" }/' sfdx-project.json
    git init -q -b main .
    git add -A
    GIT_AUTHOR_DATE='2026-01-05T10:00:00Z' GIT_COMMITTER_DATE='2026-01-05T10:00:00Z' \
        git commit -q -m 'Import the project'
    git switch -q -c round-circle
    sed -i 's/radius \* radius, 2)/radius * radius, 4)/' force-app/main/default/classes/Circle.cls
    GIT_AUTHOR_DATE='2026-01-06T10:00:00Z' GIT_COMMITTER_DATE='2026-01-06T10:00:00Z' \
        git commit -q -am 'Round the area of a circle to four places'
}

# Three characters per frame at about 30 characters a second: it reads as
# typing and keeps the recording, and so the SVG, small.
type_out() {
    local s=$1 i
    for ((i = 0; i < ${#s}; i += 3)); do
        printf '%s' "${s:i:3}"
        sleep 0.1
    done
}
note() { printf '\033[2m# %s\033[0m\n' "$1"; sleep 0.7; }
run() {
    printf '\033[36m/work\033[0m $ '
    type_out "$1"
    sleep 0.4
    printf '\n'
    eval "$1"
    sleep "${2:-1.2}"
}

setup >/dev/null 2>&1
cd "$work"
clear
sleep 0.5

note 'A pull request changes one Apex class.'
run 'git log --oneline --decorate'
note 'Read every declaration and reference in the project.'
run 'sf-impact-extract --root . --out refs.pack'
note 'Compare the head with where it parts from main.'
run 'sf-impact-delta --merge-base main --head HEAD --out delta.jsonl'
note 'Map the difference onto symbols.'
run 'sf-impact-changed --delta delta.jsonl --refs refs.pack --out changed.jsonl'
run "jq -c 'select(.row == \"symbol\") | {symbol, change}' changed.jsonl"
note 'Walk the references and select the Apex tests to run.'
run 'sf-impact-tests --refs refs.pack --changed changed.jsonl --out impact.json'
clear
note 'Render the pull request comment.'
run 'sf-impact-render --impact impact.json --format md | head -n 26' 4
# A last output event, so that the recording holds the final screen for a moment.
printf '\033[0m'
