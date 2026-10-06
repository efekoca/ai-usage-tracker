#!/bin/sh
# dpkg passes "upgrade" and rpm passes 1 while a newer version replaces this one; keep the file then
case "$1" in
  upgrade|1) exit 0 ;;
esac
rm -f /usr/share/applications/ai-usage-tracker.desktop
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q /usr/share/applications || true
exit 0
