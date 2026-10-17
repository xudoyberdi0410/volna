#!/bin/sh
# Runs xtool in ios/ with what it needs on PATH: when there is no `zip`
# (Info-ZIP), the stand-in next to this script packs the .ipa.
#
#   ios/scripts/xtool.sh dev                      # build, sign, install, run on the phone
#   ios/scripts/xtool.sh dev build -c release --ipa
here="$(cd "$(dirname "$0")" && pwd)"
command -v zip >/dev/null 2>&1 || PATH="$here:$PATH"
export PATH
cd "$here/.." || exit 1
exec xtool "$@"
