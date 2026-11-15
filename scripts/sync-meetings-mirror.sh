#!/bin/sh
# Mirrors another Mac's Voinc meetings folder for a local, read-only `voinc mcp
# --meetings-dir`. One rsync over SSH; audio (*.wav), lock, and temporary
# files stay behind. Updated files land together at the end of the transfer
# (--delay-updates), and meetings trashed on the source disappear here.
#
# Exits quietly when the source is asleep or unreachable, so a LaunchAgent can
# run it every few minutes. The mirror and its meeting folders stay owner-only. See scripts/launchd/voinc-meetings-mirror.plist.
#
# Environment:
#   VOINC_MEETINGS_HOST    SSH host (default: pro)
#   VOINC_MEETINGS_SOURCE  folder on the host, relative to its home
#                        (default: Library/Application Support/voinc/meetings)
#   VOINC_MEETINGS_MIRROR  local mirror (default: ~/Library/Caches/voinc-meetings-mirror)
set -eu

host=${VOINC_MEETINGS_HOST:-pro}
source_dir=${VOINC_MEETINGS_SOURCE:-Library/Application Support/voinc/meetings}
mirror=${VOINC_MEETINGS_MIRROR:-$HOME/Library/Caches/voinc-meetings-mirror}

mkdir -p "$mirror"
chmod 700 "$mirror"

# The remote shell splits the path, so escape its spaces.
remote_path=$(printf '%s/' "${source_dir%/}" | sed 's/ /\\ /g')
errors=$(mktemp "${TMPDIR:-/tmp}/voinc-meetings-mirror.XXXXXX")
trap 'rm -f "$errors"' EXIT

status=0
rsync -a --delete --delete-delay --delay-updates --timeout=60 \
	--exclude='*.wav' --exclude='*.lock' --exclude='*.tmp' --exclude='.DS_Store' \
	-e 'ssh -o BatchMode=yes -o ConnectTimeout=5 -o ServerAliveInterval=10 -o ServerAliveCountMax=2' \
	"$host:$remote_path" "$mirror/" 2>"$errors" || status=$?

# rsync -a copies the source folder's mode onto the mirror root.
chmod 700 "$mirror"
if [ "$status" -eq 0 ]; then
	exit 0
fi
# 255 from ssh, or a connection-level rsync failure: the host is asleep,
# offline, or refused the connection. Try again on the next run.
if [ "$status" -eq 255 ] || grep -qiE 'connect|timed out|timeout|refused|unreachable|no route|resolve|broken pipe|reset by peer|closed' "$errors"; then
	exit 0
fi
cat "$errors" >&2
exit "$status"
