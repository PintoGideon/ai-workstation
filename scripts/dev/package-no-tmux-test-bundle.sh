#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat >&2 <<'EOF'
Usage: package-no-tmux-test-bundle.sh <aiw-binary> <output-archive>

Builds a runnable test archive for the repo's no-tmux Docker image.
It wraps the provided aiw binary with its runtime loader and shared
libraries, then copies the already-bundled tmux directory from the
current tmux installation.
EOF
  exit 1
}

[ $# -eq 2 ] || usage

aiw_bin="$1"
out_archive="$2"

[ -f "$aiw_bin" ] || {
  echo "aiw binary not found: $aiw_bin" >&2
  exit 1
}

tmux_bin="$(command -v tmux || true)"
[ -n "$tmux_bin" ] || {
  echo "tmux not found in PATH" >&2
  exit 1
}

tmux_bundle_dir="$(cd "$(dirname "$tmux_bin")" && pwd)"
if [[ ! -f "$tmux_bundle_dir/tmux" ]]; then
  echo "expected tmux wrapper next to tmux binary: $tmux_bundle_dir/tmux" >&2
  exit 1
fi

bundle_name="$(basename "${out_archive%.tar.gz}")"
work_dir="$(mktemp -d)"
bundle_dir="$work_dir/$bundle_name"

cleanup() {
  rm -rf "$work_dir"
}
trap cleanup EXIT

mkdir -p "$bundle_dir"

copy_tree() {
  local src="$1"
  local dest="$2"
  mkdir -p "$(dirname "$dest")"
  cp -a "$src" "$dest"
}

aiw_root="$bundle_dir/aiw-root"
mkdir -p "$aiw_root"
cp "$aiw_bin" "$bundle_dir/aiw-real"
chmod +x "$bundle_dir/aiw-real"

mapfile -t aiw_deps < <(
  ldd "$aiw_bin" \
    | awk '{for (i = 1; i <= NF; i++) if ($i ~ /^\//) print $i}' \
    | sed 's/[()]//g' \
    | sort -u
)

if [[ ${#aiw_deps[@]} -eq 0 ]]; then
  echo "failed to resolve runtime dependencies for $aiw_bin" >&2
  exit 1
fi

aiw_loader=""
aiw_lib_dirs=()
for dep in "${aiw_deps[@]}"; do
  if [[ "$dep" == *ld-linux* || "$dep" == *ld-musl* ]]; then
    aiw_loader="$dep"
  else
    aiw_lib_dirs+=("$(dirname "$dep")")
  fi
  copy_tree "$dep" "$aiw_root$dep"
done

aiw_lib_dirs=($(printf '%s\n' "${aiw_lib_dirs[@]}" | sort -u))

cat >"$bundle_dir/aiw" <<EOF
#!/usr/bin/env bash
set -euo pipefail
HERE="\$(cd "\$(dirname "\$0")" && pwd)"

aiw_root="\$HERE/aiw-root"
aiw_real="\$HERE/aiw-real"
loader="\$aiw_root${aiw_loader}"
lib_dirs=(
$(for dir in "${aiw_lib_dirs[@]}"; do printf '  "%s/%s"\n' '$aiw_root' "${dir#/}"; done)
)

if [[ -x "\$loader" && \${#lib_dirs[@]} -gt 0 ]]; then
  ld_path="\$(IFS=:; echo "\${lib_dirs[*]}")"
  exec "\$loader" --library-path "\$ld_path" "\$aiw_real" "\$@"
fi

if [[ \${#lib_dirs[@]} -gt 0 ]]; then
  ld_path="\$(IFS=:; echo "\${lib_dirs[*]}")"
  export LD_LIBRARY_PATH="\$ld_path\${LD_LIBRARY_PATH:+:\$LD_LIBRARY_PATH}"
fi

exec "\$aiw_real" "\$@"
EOF
chmod +x "$bundle_dir/aiw"

cp -a "$tmux_bundle_dir/." "$bundle_dir/"

tar -C "$work_dir" -czf "$out_archive" "$bundle_name"
echo "Created $out_archive"
