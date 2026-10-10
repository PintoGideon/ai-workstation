#!/usr/bin/env bash
set -euo pipefail

install_root="${AIW_INSTALL_ROOT:-/opt/aiw}"
# AIW publishes no release bundles: set AIW_RELEASE_ARCHIVE (a local bundle)
# or AIW_RELEASE_BASE (where your own bundles are hosted).
release_base="${AIW_RELEASE_BASE:-}"
override_archive="${AIW_RELEASE_ARCHIVE:-}"

log() {
    printf '[aiw-no-tmux] %s\n' "$*"
}

bundle_name_for_arch() {
    case "$(uname -m)" in
        x86_64|amd64)
            printf '%s\n' "aiw-x86_64-unknown-linux-musl"
            ;;
        aarch64|arm64)
            printf '%s\n' "aiw-aarch64-unknown-linux-gnu"
            ;;
        *)
            printf 'Unsupported architecture: %s\n' "$(uname -m)" >&2
            return 1
            ;;
    esac
}

archive_path_for_install() {
    if [[ -n "$override_archive" ]]; then
        printf '%s\n' "$override_archive"
        return 0
    fi

    printf '%s/%s.tar.gz\n' "$release_base" "$(bundle_name_for_arch)"
}

download_archive() {
    local archive="$1"
    local source_path="${AIW_RELEASE_ARCHIVE:-}"

    if [[ -n "$source_path" ]]; then
        if [[ ! -f "$source_path" ]]; then
            printf 'AIW_RELEASE_ARCHIVE does not exist: %s\n' "$source_path" >&2
            return 1
        fi
        cp "$source_path" "$archive"
        return 0
    fi

    if [[ -z "$release_base" ]]; then
        printf 'Set AIW_RELEASE_ARCHIVE or AIW_RELEASE_BASE: AIW publishes no release bundles\n' >&2
        return 1
    fi
    curl -fsSL "$(archive_path_for_install)" -o "$archive"
}

main() {
    local archive bundle_name release_root bundle_dir tempdir=""

    if command -v tmux >/dev/null 2>&1; then
        log "tmux is already available in PATH before installation"
    else
        log "tmux is not installed in the base image"
    fi

    tempdir="$(mktemp -d)"
    trap 'rm -rf "${tempdir:-}"' EXIT

    archive="$tempdir/aiw.tar.gz"
    download_archive "$archive"

    mapfile -t archive_entries < <(tar -tzf "$archive")
    bundle_name=""
    if [[ ${#archive_entries[@]} -gt 0 ]]; then
        bundle_name="${archive_entries[0]%%/*}"
    fi
    if [[ -z "$bundle_name" ]]; then
        printf 'Failed to determine bundle directory from %s\n' "$archive" >&2
        return 1
    fi

    release_root="$install_root/releases"
    bundle_dir="$release_root/$bundle_name"

    mkdir -p "$release_root"
    rm -rf "$bundle_dir"
    tar -xzf "$archive" -C "$release_root"

    if [[ ! -x "$bundle_dir/aiw" ]]; then
        printf 'Installed bundle is missing aiw: %s\n' "$bundle_dir/aiw" >&2
        return 1
    fi

    ln -sfn "$bundle_dir" "$install_root/current"

    mkdir -p "$install_root/bin"

    cat >"$install_root/bin/aiw" <<EOF
#!/usr/bin/env bash
set -euo pipefail
exec "$install_root/current/aiw" "\$@"
EOF

    cat >"$install_root/bin/tmux" <<EOF
#!/usr/bin/env bash
set -euo pipefail
exec "$install_root/current/tmux" "\$@"
EOF

    chmod 755 "$install_root/bin/aiw" "$install_root/bin/tmux"

    log "Installed AIW to $bundle_dir"
    log "Base-image tmux remained absent; the bundled tmux is now available via $install_root/bin/tmux"
}

main "$@"
