#!/bin/sh
# The shortcut portal and GNOME match the app by this file name; the menu entry stays the main one.
cat > /usr/share/applications/ai-usage-tracker.desktop <<'ENTRY'
[Desktop Entry]
Type=Application
Name=AI Usage Tracker
Comment=Shows the AI Usage Tracker widget
Exec=ai-usage-tracker --toggle-widget
Icon=ai-usage-tracker
Terminal=false
Categories=Utility;
NoDisplay=true
ENTRY
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q /usr/share/applications || true
exit 0
