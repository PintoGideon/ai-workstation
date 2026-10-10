#!/bin/bash
# Write a custom session status into the AIW database.
#
# AIW sets AIW_SESSION_ID and AIW_STATUS_DIR when launching custom sessions.
# Call this script from your session's hook to report a one-line status shown
# in the AIW dashboard.
#
# Usage: set-session-status.sh "status text"

if [ "${AIW_ACTIVE:-}" != "1" ]; then
    exit 0
fi

STATUS_TEXT="${1:-}"
SESSION_ID="${AIW_SESSION_ID:-}"

if [ -z "$SESSION_ID" ] || [ -z "$STATUS_TEXT" ]; then
    exit 0
fi

if command -v aiw >/dev/null 2>&1; then
    aiw set-status "$SESSION_ID" "$STATUS_TEXT" 2>/dev/null && exit 0
fi

# Fallback: write to the legacy file path when aiw is not in PATH.
if [ -n "$AIW_STATUS_DIR" ]; then
    mkdir -p "$AIW_STATUS_DIR"
    printf '%s\n' "$STATUS_TEXT" > "$AIW_STATUS_DIR/${SESSION_ID}.txt"
fi
