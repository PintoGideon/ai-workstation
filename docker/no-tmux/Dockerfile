FROM debian:bookworm-slim

ENV DEBIAN_FRONTEND=noninteractive \
    HOME=/home/aiw \
    LANG=C.UTF-8 \
    TERM=xterm-256color \
    COLORTERM=truecolor \
    AIW_INSTALL_ROOT=/opt/aiw

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        bash \
        ca-certificates \
        curl \
        git \
        jq \
        less \
        nodejs \
        npm \
        procps \
        tar \
    && npm install -g @anthropic-ai/claude-code \
    && rm -rf /var/lib/apt/lists/*

RUN useradd --create-home --shell /bin/bash aiw \
    && mkdir -p /opt/aiw \
    && chown -R aiw:aiw /opt/aiw /home/aiw

WORKDIR /home/aiw

COPY docker/no-tmux/install-aiw-release.sh /usr/local/bin/aiw-install-release
COPY docker/no-tmux/entrypoint.sh /usr/local/bin/aiw-no-tmux-entrypoint

RUN chmod 755 \
        /usr/local/bin/aiw-install-release \
        /usr/local/bin/aiw-no-tmux-entrypoint

USER aiw

ENTRYPOINT ["/usr/local/bin/aiw-no-tmux-entrypoint"]
CMD ["bash"]
