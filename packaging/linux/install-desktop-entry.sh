#!/usr/bin/env bash
# Adds Capybara's D2 Editor to your desktop's application menu, for your user
# only (no root): writes ~/.local/share/applications/capybaras-d2-editor.desktop
# pointing at the CapybarasD2Editor binary next to this script, with the
# capybara icon from this folder.
#
#   ./install-desktop-entry.sh            add or update the menu entry
#   ./install-desktop-entry.sh --remove   remove it again
#
# Run it again after moving this folder. Nothing else is installed or changed;
# the editor keeps its data in capybara/ next to the binary (or in CAPY_HOME).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
apps="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
entry="$apps/capybaras-d2-editor.desktop"

if [[ "${1:-}" == "--remove" ]]; then
    rm -f -- "$entry"
    echo "removed $entry"
    exit 0
elif [[ $# -gt 0 ]]; then
    echo "usage: $0 [--remove]" >&2
    exit 2
fi

bin="$here/CapybarasD2Editor"
icon="$here/capybara.png"
template="$here/capybaras-d2-editor.desktop"
for f in "$bin" "$icon" "$template"; do
    [[ -e "$f" ]] || { echo "error: $f is missing" >&2; exit 1; }
done
# The Exec key quotes the path; keep it free of characters that need
# escaping there (desktop entry spec, "The Exec key").
case "$here" in
    *[\"\`\$\\]* | *$'\n'*)
        echo "error: the folder path contains \" \` \$ \\ or a newline; move the folder and run this again" >&2
        exit 1
        ;;
esac

mkdir -p -- "$apps"
tmp="$(mktemp "$apps/.capybaras-d2-editor.XXXXXX")"
trap 'rm -f -- "$tmp"' EXIT
# Escape what sed treats specially in a replacement (the | delimiter and &).
sed_esc() { printf '%s' "$1" | sed -e 's/[|&]/\\&/g'; }
sed -e "s|^Exec=.*|Exec=\"$(sed_esc "$bin")\"|" \
    -e "s|^Icon=.*|Icon=$(sed_esc "$icon")\nPath=$(sed_esc "$here")|" \
    "$template" > "$tmp"
chmod 644 "$tmp"
mv -f -- "$tmp" "$entry"
trap - EXIT
echo "added $entry"
echo "  Exec=\"$bin\""
