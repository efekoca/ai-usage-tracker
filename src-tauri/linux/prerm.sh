#!/bin/sh
# Like the Windows uninstaller: undo each user's opt-in edits to Claude Code's settings.json and
# drop the login entry while the program is still here. dpkg passes "remove", rpm passes 0.
set -e
case "$1" in
  remove|0) ;;
  *) exit 0 ;;
esac

# The app's data folder and login entry follow XDG_DATA_HOME and XDG_CONFIG_HOME, which only the
# user's session knows: read the variable from their running app, else their systemd user manager.
session_dir() {
  found=
  # only a signed-in user has a session to ask
  [ -d "/run/user/$2" ] || return 0
  for pid in $(pgrep -u "$2" -f '^(/usr/bin/)?ai-usage-tracker( |$)' 2>/dev/null); do
    found=$(tr '\0' '\n' <"/proc/$pid/environ" 2>/dev/null | sed -n "s/^$3=//p" | head -n 1)
    if [ -n "$found" ]; then break; fi
  done
  if [ -z "$found" ]; then
    found=$(runuser -u "$1" -- env -i PATH=/usr/bin:/bin XDG_RUNTIME_DIR="/run/user/$2" timeout 5 systemctl --user show-environment 2>/dev/null | sed -n "s/^$3=//p" | head -n 1)
  fi
  case "$found" in /*) printf '%s\n' "$found" ;; esac
}

getent passwd | while IFS=: read -r user _ uid _ _ home _; do
  [ "$uid" -ge 1000 ] 2>/dev/null && [ "$uid" -lt 60000 ] || continue
  data=$(session_dir "$user" "$uid" XDG_DATA_HOME)
  config=$(session_dir "$user" "$uid" XDG_CONFIG_HOME)
  # only users who ran the app; the revert would otherwise create its data folder
  dir=
  for d in ${data:+"$data"} "$home/.local/share"; do
    if [ -d "$d/AIUsageTracker" ]; then dir=$d; break; fi
  done
  [ -n "$dir" ] || continue
  runuser -u "$user" -- env -i HOME="$home" XDG_DATA_HOME="$dir" PATH=/usr/bin:/bin timeout 30 /usr/bin/ai-usage-tracker --revert-capture </dev/null \
    || echo "ai-usage-tracker: could not undo the Claude Code settings changes for $user; see $home/.claude/settings.json" >&2
  runuser -u "$user" -- rm -f "$home/.config/autostart/ai-usage-tracker.desktop" ${config:+"$config/autostart/ai-usage-tracker.desktop"} || true
done
exit 0
