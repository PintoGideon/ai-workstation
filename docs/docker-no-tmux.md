# Docker No-Tmux Image

This image is for testing AIW installation in a container that does not
preinstall the system `tmux` package.

It installs the latest AIW release bundle into `/opt/aiw`, installs
Claude Code into the image, then exposes wrapper commands in
`/opt/aiw/bin` that point at the bundled `aiw` and `tmux` binaries
from that release.

For local testing of a checkout, use
[`scripts/dev/package-no-tmux-test-bundle.sh`](../scripts/dev/package-no-tmux-test-bundle.sh)
to create a runnable archive from the current `aiw` binary plus the
existing bundled `tmux` installation. That avoids glibc mismatches
when running the bundle inside the Debian-based no-tmux image.

## Build

```bash
docker build -f docker/no-tmux/Dockerfile -t aiw-no-tmux .
```

## Run

```bash
scripts/dev/run-no-tmux-docker.sh -- bash
```

The container starts with a shell after installing AIW. Inside it, you
can verify that the base image did not ship `tmux`, that `claude`
is available, and that AIW works from the bundled release:

```bash
command -v tmux || true
command -v claude
claude --version
aiw -V
tmux -V
```

## Custom Release

You can point the installer at a different archive without changing the
image:

```bash
AIW_RELEASE_ARCHIVE="$PWD/aiw.tar.gz" scripts/dev/run-no-tmux-docker.sh
```

To test the current checkout, build a local bundle first:

```bash
cargo build --release
scripts/dev/package-no-tmux-test-bundle.sh target/release/aiw /tmp/aiw-no-tmux-test.tar.gz
AIW_RELEASE_ARCHIVE=/tmp/aiw-no-tmux-test.tar.gz scripts/dev/run-no-tmux-docker.sh -- bash
```

Or point it at a directory or server that hosts your own bundles (AIW publishes
no release bundles):

```bash
AIW_RELEASE_BASE=https://bundles.example.internal/aiw \
  scripts/dev/run-no-tmux-docker.sh
```

Set `AIW_SKIP_INSTALL=1` if you want the container to start without
running the installer.
