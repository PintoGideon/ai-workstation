#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
IMAGE_NAME="${AIW_NO_TMUX_IMAGE:-aiw-no-tmux}"
CONTAINER_NAME="${AIW_NO_TMUX_CONTAINER:-aiw-no-tmux}"

usage() {
    cat <<EOF
Usage: $(basename "$0") [docker run args...]

Builds the no-tmux AIW image, then runs it interactively.

Pass Docker run flags before \`--\` and the container command after it.

Environment variables:
  AIW_NO_TMUX_IMAGE      Docker image name to build/run (default: $IMAGE_NAME)
  AIW_NO_TMUX_CONTAINER  Docker container name to use (default: $CONTAINER_NAME)
  AIW_RELEASE_ARCHIVE    Optional local release tarball path for the container
  AIW_RELEASE_BASE       Optional release download base URL

Examples:
  $0
  $0 --rm -it -- bash
  AIW_RELEASE_ARCHIVE=$PWD/aiw.tar.gz $0
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
    usage
    exit 0
fi

docker_run_args=()
container_cmd=()
docker_env_args=()
docker_tty_args=()
parsing_run_args=1

for arg in "$@"; do
    if [[ "$arg" == "--" ]]; then
        parsing_run_args=0
        continue
    fi

    if [[ "$parsing_run_args" -eq 1 ]]; then
        docker_run_args+=("$arg")
    else
        container_cmd+=("$arg")
    fi
done

for var in AIW_RELEASE_ARCHIVE AIW_RELEASE_BASE AIW_INSTALL_ROOT AIW_SKIP_INSTALL; do
    if [[ -n "${!var:-}" ]]; then
        docker_env_args+=("-e" "$var=${!var}")
    fi
done

if [[ -n "${AIW_RELEASE_ARCHIVE:-}" && -f "${AIW_RELEASE_ARCHIVE:-}" ]]; then
    docker_run_args+=("-v" "${AIW_RELEASE_ARCHIVE}:${AIW_RELEASE_ARCHIVE}:ro")
fi

if [[ -t 0 && -t 1 ]]; then
    docker_tty_args=(-it)
fi

docker build \
    -f "$REPO_ROOT/docker/no-tmux/Dockerfile" \
    -t "$IMAGE_NAME" \
    "$REPO_ROOT"

exec docker run \
    --rm \
    --name "$CONTAINER_NAME" \
    "${docker_tty_args[@]}" \
    "${docker_env_args[@]}" \
    "${docker_run_args[@]}" \
    "$IMAGE_NAME" \
    "${container_cmd[@]}"
