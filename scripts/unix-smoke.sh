#!/usr/bin/env bash
# End-to-end smoke test of the ProfilePort CLI on Linux and macOS.
# Counterpart of scripts/windows-smoke.ps1.
#
# Usage: scripts/unix-smoke.sh target/release/profileport-cli

set -u
EXE="$(cd "$(dirname "$1")" && pwd)/$(basename "$1")"
root="$(mktemp -d)"
src="$root/home/anna"
drive="$root/E"
dst="$root/home/anna.smith"
pass=0
fail=0

check() { # check <description> <command...>
    local what="$1"
    shift
    if "$@" >/dev/null 2>&1; then
        echo "  PASS  $what"
        pass=$((pass + 1))
    else
        echo "  FAIL  $what"
        fail=$((fail + 1))
    fi
}
mask() { sed -E 's/[0-9A-Z]{4}-[0-9A-Z]{4}-[0-9A-Z]{4}/<CODE>/g'; }
mtime() { stat -c %Y "$1" 2>/dev/null || stat -f %m "$1"; }
contains() { grep -qE "$2" <<<"$1"; }
same_text() { [ "$(cat "$1")" = "$2" ]; }

echo "== Building test profile in $src"
mkdir -p "$src/Documents/Deep/Deeper" "$src/Pictures" "$src/.cache/thumbs" "$src/.config/app" "$drive" "$dst"
printf 'quarterly numbers' >"$src/Documents/report.txt"
printf 'unicode' >"$src/Documents/Ærø – 日本.txt"
printf 'colon' >"$src/Documents/notes: draft.txt"
printf 'deep' >"$src/Documents/Deep/Deeper/deep.txt"
printf '[ui]' >"$src/.config/app/settings.ini"
printf 'cache' >"$src/.cache/thumbs/x.png"
head -c 3000000 /dev/urandom >"$src/Pictures/big.raw"
ln -s Documents "$src/docs-link"
touch -t 202001021234 "$src/Documents/report.txt"

echo "== Export"
out="$("$EXE" export "$src" --drive "$drive" --ticket 2001 --computer SMOKE --no-confirm </dev/null 2>&1)"
ec=$?
mask <<<"$out"
code="$(grep -oE '[0-9A-Z]{4}-[0-9A-Z]{4}-[0-9A-Z]{4}' <<<"$out" | head -1)"
check "export completes cleanly (exit 0)" test "$ec" -eq 0
check "transfer code is shown" test -n "$code"
check "symlink recorded, not followed" contains "$out" '1 link\(s\) recorded'
check "cache folder excluded by rule" contains "$out" 'cache/temp location'
pkg="$(ls -d "$drive"/ProfilePort/transfers/2001-*)"

echo "== List (no code)"
list="$("$EXE" list --drive "$drive" 2>&1)"
echo "$list"
check "listing shows the ticket as complete" contains "$list" '2001 .* complete'

echo "== Verify"
"$EXE" verify "$pkg" --code "$code" </dev/null | mask
check "verify succeeds" test "${PIPESTATUS[0]}" -eq 0

echo "== Restore to a different user"
rout="$("$EXE" restore "$pkg" --to "$dst" --code "$code" </dev/null 2>&1)"
rec=$?
mask <<<"$rout"
check "restore fully successful (exit 0)" test "$rec" -eq 0
check "large file identical" cmp -s "$src/Pictures/big.raw" "$dst/Pictures/big.raw"
check "nested file restored" same_text "$dst/Documents/Deep/Deeper/deep.txt" deep
check "Unicode name restored" same_text "$dst/Documents/Ærø – 日本.txt" unicode
check "name with ':' kept on this system" same_text "$dst/Documents/notes: draft.txt" colon
check "hidden config restored" same_text "$dst/.config/app/settings.ini" '[ui]'
check "excluded cache not restored" test ! -e "$dst/.cache/thumbs/x.png"
check "link not turned into a copy" test ! -e "$dst/docs-link"
check "modified time preserved" test "$(mtime "$src/Documents/report.txt")" = "$(mtime "$dst/Documents/report.txt")"
check "no temporary files left behind" test -z "$(find "$dst" -name '.profileport-*')"

echo "== Restore again (as after an interrupted restore)"
again="$("$EXE" restore "$pkg" --to "$dst" --code "$code" </dev/null 2>&1)"
mask <<<"$again"
check "second restore recognises files already in place" contains "$again" 'already there with identical content'
check "second restore reports no conflicts" test -z "$(grep 'Name conflicts' <<<"$again")"

echo "== Wipe"
# A second backup, so we can check a wipe touches only its own package.
"$EXE" export "$src/Documents" --drive "$drive" --ticket 2002 --computer SMOKE --no-confirm </dev/null >/dev/null 2>&1
pkg2="$(ls -d "$drive"/ProfilePort/transfers/2002-*)"
wrong="$(echo 9999 | "$EXE" wipe "$pkg" 2>&1)"
wec=$?
check "wipe with the wrong ticket typed is refused (exit 2)" test "$wec" -eq 2
check "refused wipe leaves the backup in place" test -f "$pkg/header.bin"
printf 'notes' >"$pkg/packs/stray.txt"
stray="$(echo 2001 | "$EXE" wipe "$pkg" 2>&1)"
check "wipe refuses a folder with files that aren't part of a backup" contains "$stray" 'not part of a ProfilePort package'
check "refused wipe leaves the stray file alone" test -f "$pkg/packs/stray.txt"
rm "$pkg/packs/stray.txt"
wout="$(echo 2001 | "$EXE" wipe "$pkg" --overwrite 2>&1)"
wec=$?
echo "$wout"
check "wipe succeeds (exit 0)" test "$wec" -eq 0
check "wipe reports the key destroyed" contains "$wout" 'key destroyed'
check "wiped backup folder is gone" test ! -e "$pkg"
check "other backup untouched by the wipe" test -f "$pkg2/header.bin"
check "restored files untouched by the wipe" same_text "$dst/Documents/Deep/Deeper/deep.txt" deep
check "source untouched by the wipe" same_text "$src/Documents/report.txt" 'quarterly numbers'
dout="$(echo WIPE | "$EXE" wipe --drive "$drive" --all 2>&1)"
check "wipe --all clears the drive" test -z "$(ls -A "$drive/ProfilePort/transfers")"
check "list shows no backups after wipe --all" contains "$("$EXE" list --drive "$drive" 2>&1)" 'No backups'

echo "== Profiles"
"$EXE" profiles
check "profiles command runs" "$EXE" profiles

echo
echo "$pass passed, $fail failed"
if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
    echo "## $(uname -s) smoke test: $pass passed, $fail failed" >>"$GITHUB_STEP_SUMMARY"
fi
rm -rf "$root"
[ "$fail" -eq 0 ]
