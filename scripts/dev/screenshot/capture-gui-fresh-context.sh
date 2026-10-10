#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
export AIW_GUI_FRESH_CONTEXT=1
export AIW_GUI_CAPTURE_RUNNER="$SCRIPT_DIR/capture-gui-session-sidebar.py"
export AIW_GUI_CAPTURE_FRAMES="$SCRIPT_DIR/capture-gui-fresh-context-frames.py"
bash "$SCRIPT_DIR/capture-gui-diff.sh" "${1:?Pass an output directory}"
