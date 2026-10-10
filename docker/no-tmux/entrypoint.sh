#!/usr/bin/env bash
set -euo pipefail

if [[ "${AIW_SKIP_INSTALL:-0}" != "1" ]]; then
    /usr/local/bin/aiw-install-release
fi

export PATH="${AIW_INSTALL_ROOT:-/opt/aiw}/bin:${PATH}"

exec "$@"
