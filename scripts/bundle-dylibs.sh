#!/usr/bin/env bash
# SPDX-License-Identifier: MIT OR Apache-2.0
# bundle-dylibs.sh - self-contain a macOS .app's non-system dylib dependencies.
#
# Usage:
#     bundle-dylibs.sh <macos_dir> <root> [<root> ...]
#
# <macos_dir>   the bundle's Contents/MacOS directory (where the binaries live).
# <root>...     binaries/dylibs already inside <macos_dir> whose dependency graph
#               should be bundled (e.g. cdj3k-emu, qemu-img, socket_vmnet,
#               libcdj3k-emu-qemu.dylib).
#
# For each root it walks the transitive `otool -L` dependency graph, copies every
# NON-system dylib (anything not under /usr/lib or /System - i.e. Homebrew
# /opt/homebrew, /usr/local, etc.) flat into <macos_dir>, and rewrites every load
# command (and each copied dylib's own id) to @loader_path/<basename> so the app
# runs on a machine without Homebrew. Each Mach-O it edits is ad-hoc re-signed,
# since editing load commands invalidates any existing signature; bundle.sh's
# codesign pass re-seals everything with the final identity afterward.
#
# System libraries (/usr/lib, /System) are left untouched - they exist on every
# Mac. Deps already expressed as @loader_path/<name> pointing at a file already in
# <macos_dir> are treated as bundled and skipped (idempotent on re-runs).
#
# Needs only what ships with the Xcode Command Line Tools: otool,
# install_name_tool, codesign. Runs on the stock /bin/bash 3.2.
set -euo pipefail

if [[ $# -lt 2 ]]; then
    sed -n '3,/^$/p' "$0" | sed 's/^# \{0,1\}//' >&2
    exit 2
fi

MACOS_DIR="$(cd "$1" && pwd)"
shift

is_system() { [[ "$1" == /usr/lib/* || "$1" == /System/* ]]; }

# LC_ID_DYLIB install name of a dylib; empty for a plain executable.
otool_id() { otool -D "$1" 2>/dev/null | sed -n '2p'; }

# Dependency install names from `otool -L` (the first line is the file itself).
otool_deps() { otool -L "$1" | tail -n +2 | awk '{print $1}'; }

# LC_RPATH entries of a Mach-O, one per line.
otool_rpaths() {
    otool -l "$1" | awk '/ cmd LC_RPATH/{r=1; next} r && / path /{print $2; r=0}'
}

# in_list <needle> <haystack...>
in_list() {
    local x="$1"; shift
    local e
    for e in "$@"; do [[ "$e" == "$x" ]] && return 0; done
    return 1
}

# resolve <install_name> <owner> <origin_dir>: print the source file for a
# dependency, or nothing if it cannot be found.
#
# origin_dir is the directory the owner was *copied from* (its own dir for
# roots). @loader_path / @rpath siblings are resolved there - Homebrew dylibs
# routinely name a sibling @rpath/libfoo without carrying an rpath of their own
# (they rely on the loading executable's rpath), and the sibling lives next to
# the original, not next to the copy.
resolve() {
    local dep="$1" owner="$2" origin="$3" tail rp cand
    case "$dep" in
        @loader_path/*)     echo "$origin/${dep#@loader_path/}" ;;
        @executable_path/*) echo "$MACOS_DIR/${dep#@executable_path/}" ;;
        @rpath/*)
            tail="${dep#@rpath/}"
            while IFS= read -r rp; do
                [[ -n "$rp" ]] || continue
                rp="${rp//@loader_path/$origin}"
                rp="${rp//@executable_path/$MACOS_DIR}"
                cand="$rp/$tail"
                if [[ -e "$cand" ]]; then echo "$cand"; return 0; fi
            done < <(otool_rpaths "$owner")
            # Fallback: the sibling next to the original (most Homebrew @rpath deps).
            cand="$origin/$tail"
            [[ -e "$cand" ]] && echo "$cand"
            ;;
        *) echo "$dep" ;;   # absolute path
    esac
}

roots=()
for r in "$@"; do
    if [[ ! -e "$MACOS_DIR/$r" ]]; then
        echo "bundle-dylibs: root not found: $MACOS_DIR/$r" >&2
        exit 1
    fi
    roots+=("$MACOS_DIR/$r")
done

# Work list of "file<TAB>origin_dir" entries, consumed by index so entries can be
# appended while iterating.  processed/bundled are plain lists (bash 3.2 has no
# associative arrays); rewrites is a "file<TAB>old<TAB>new" line file.
queue=()
for r in "${roots[@]}"; do queue+=("$r	$(dirname "$r")"); done
processed=()
bundled=()          # basenames now living in MACOS_DIR
REWRITES="$(mktemp -t bundle-dylibs)"
trap 'rm -f "$REWRITES"' EXIT

i=0
while [[ $i -lt ${#queue[@]} ]]; do
    f="${queue[$i]%%	*}"
    origin="${queue[$i]#*	}"
    i=$((i + 1))
    in_list "$f" "${processed[@]+"${processed[@]}"}" && continue
    processed+=("$f")
    own_id="$(otool_id "$f")"

    while IFS= read -r dep; do
        [[ -n "$dep" ]] || continue
        is_system "$dep" && continue
        [[ -n "$own_id" && "$dep" == "$own_id" ]] && continue   # the dylib's own id
        base="$(basename "$dep")"
        new_name="@loader_path/$base"
        # Already-bundled style: @loader_path/<name> with the file present.
        if [[ "$dep" == "$new_name" && -e "$MACOS_DIR/$base" ]]; then
            in_list "$base" "${bundled[@]+"${bundled[@]}"}" || bundled+=("$base")
            queue+=("$MACOS_DIR/$base	$MACOS_DIR")
            continue
        fi
        src="$(resolve "$dep" "$f" "$origin")"
        if [[ -z "$src" || ! -e "$src" ]]; then
            echo "  WARN: cannot resolve $dep (from $(basename "$f")) - skipping" >&2
            continue
        fi
        real_src="$(cd "$(dirname "$src")" && pwd -P)/$(basename "$src")"
        is_system "$real_src" && continue
        dest="$MACOS_DIR/$base"
        if ! in_list "$base" "${bundled[@]+"${bundled[@]}"}"; then
            if [[ "$real_src" != "$(cd "$(dirname "$dest")" 2>/dev/null && pwd -P)/$base" ]]; then
                cp "$src" "$dest"
                chmod 755 "$dest"
            fi
            bundled+=("$base")
            queue+=("$dest	$(dirname "$real_src")")
        fi
        printf '%s\t%s\t%s\n' "$f" "$dep" "$new_name" >> "$REWRITES"
    done < <(otool_deps "$f")
done

# Apply install_name_tool edits + ad-hoc re-sign every Mach-O we touched.
for f in "${processed[@]}"; do
    args=()
    base="$(basename "$f")"
    if in_list "$base" "${bundled[@]+"${bundled[@]}"}"; then   # a copied dylib: fix its own id too
        args+=(-id "@loader_path/$base")
    fi
    while IFS=$'\t' read -r file old new; do
        [[ "$file" == "$f" ]] && args+=(-change "$old" "$new")
    done < "$REWRITES"
    if [[ ${#args[@]} -gt 0 ]]; then
        install_name_tool "${args[@]}" "$f" 2>&1 | grep -v 'invalidate the code signature' || true
    fi
    codesign --force --sign - "$f" >/dev/null 2>&1
done

echo "  -> bundled ${#bundled[@]} dylib(s) into $MACOS_DIR (@loader_path, ad-hoc signed)"
