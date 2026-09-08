#!/bin/sh
# SPDX-License-Identifier: MIT OR Apache-2.0
# Refresh the cdj3k-mods submodule to the tip of a branch on its remote.
#
# The mods are developed in their own repository (github.com/nsaintot/cdj3k-mods);
# this checkout is a clone pinned by the gitlink. A commit made there reaches
# this build through a fetch, after which the new pin is committed here.
#
#   scripts/sync-mods.sh [branch]     default: the branch already checked out
set -e
SUB=guest/ep122_shim/cdj3k-mods
REPO=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
cd "$REPO"

BRANCH=${1:-$(git -C $SUB rev-parse --abbrev-ref HEAD)}
git -C $SUB fetch -q origin
git -C $SUB checkout -q -B "$BRANCH" "origin/$BRANCH"
echo "mods: $BRANCH at $(git -C $SUB log --oneline -1)"
echo "commit the new pin with: git add $SUB && git commit"
