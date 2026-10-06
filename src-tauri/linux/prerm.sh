#!/bin/sh
# Like the Windows uninstaller: undo each user's opt-in edits to Claude Code's settings.json and
# drop the login entry while the program is still here. dpkg passes "remove", rpm passes 0.
case "$1" in
  remove|0) ;;
  *) exit 0 ;;
esac
getent passwd | while IFS=: read -r user _ uid _ _ home _; do
  [ "$uid" -ge 1000 ] 2>/dev/null && [ "$uid" -lt 60000 ] || continue
  # only users who ran the app; the revert would otherwise create its data folder
  [ -d "$home/.local/share/AIUsageTracker" ] || continue
  runuser -u "$user" -- env -i HOME="$home" PATH=/usr/bin:/bin timeout 30 /usr/bin/ai-usage-tracker --revert-capture </dev/null \
    || echo "ai-usage-tracker: could not undo the Claude Code settings changes for $user; see $home/.claude/settings.json" >&2
  runuser -u "$user" -- rm -f "$home/.config/autostart/ai-usage-tracker.desktop"
done
exit 0
