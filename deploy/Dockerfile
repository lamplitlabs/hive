# A brain in a container, one per team, for a server of your own (ADR-070
# rule 6, ADR-071, docs/security.md).
#
# It runs the Hive daemon and its hub from a `hive` binary built here in
# release mode, with the console embedded. The image carries no Hive source:
# the build stage's tree stays in the build stage, and the running brain
# updates itself from published releases (`daemon.self_update release`), never
# by building. It holds no project code either (`brain.code none`): the coding
# harnesses, the project clones, the checks and the reviews run on the team's
# hands, which dial in to it.
#
# Build from the repository root:
#   docker build -f deploy/Dockerfile --build-arg HIVE_VERSION=v0.2.0 -t hive-brain .
# Run one brain (deploy/compose.yaml does this with a named volume). The hub
# listens on loopback only, and a container's loopback is its own, so the
# brain runs on the host's network; a published port would reach nothing:
#   docker run -v hive-team-a:/data --network host hive-brain
#
# Everything the brain keeps - the store, its WAL, the work dir, logs, keys -
# lives under HIVE_HOME=/data/hive on the one volume, which the host
# encrypts at rest (docs/security.md "Encryption at rest").

# ---- build: hive in release, the console embedded -------------------------
FROM rust:1.95-bookworm AS build

# Node 22 builds the console (web/hub) that crates/hive-hub embeds.
ARG NODE_MAJOR=22
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl gnupg git \
 && curl -fsSL https://deb.nodesource.com/setup_${NODE_MAJOR}.x | bash - \
 && apt-get install -y --no-install-recommends nodejs \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /src
COPY . .
# The version the binary reports (`hive version`); `daemon.self_update
# release` compares the published releases against it. .dockerignore keeps
# .git out of the context, so crates/hive/build.rs cannot read a tag: give
# the release tag the image is built from (deploy/brain.sh build does).
ARG HIVE_VERSION
RUN test -n "${HIVE_VERSION}" || { echo "build with --build-arg HIVE_VERSION=vX.Y.Z" >&2; exit 1; }
ENV HIVE_VERSION=${HIVE_VERSION}
RUN npm --prefix web/hub ci --no-audit --no-fund \
 && cargo build --release --locked --bin hive \
 && install -m 755 target/release/hive /hive \
 && /hive version

# ---- runtime: the binary, git, gh, and nothing of the source ---------------
FROM debian:bookworm-slim

# git: the daemon reads and lands branches the hands push; gh: GitHub reads,
# pull requests and merges; tini: signals reach the daemon and the hub, and
# their children are reaped.
# openssh-client: ssh-keygen, which the brain runs to give each run its
# signing key, to check commit signatures and to check the signature of a
# machine that dials in (and ssh, for git remotes that use it). git only
# recommends it, so it is named here.
# openssl: the brain signs its GitHub App tokens with the openssl command.
# No coding harness (copilot, claude, codex): those run on hands. The
# built-in harness the brain's own lead uses is `hive`.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl git gnupg openssh-client openssl tini \
 && install -d -m 0755 /etc/apt/keyrings \
 && curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
      -o /etc/apt/keyrings/githubcli-archive-keyring.gpg \
 && echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
      > /etc/apt/sources.list.d/github-cli.list \
 && apt-get update \
 && apt-get install -y --no-install-recommends gh \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --create-home --uid 10001 --shell /bin/sh hive \
 && install -d -o hive -g hive /data /home/hive/.hive/bin

# The binary lives where `daemon.self_update release` replaces it: a path the
# hive user owns, beside which it writes hive.new and keeps hive.prev. A
# release installed there survives a container restart only while the
# container lives; a new image is the durable update (docs/security.md "Updates").
COPY --from=build --chown=hive:hive /hive /home/hive/.hive/bin/hive
COPY --chown=hive:hive deploy/entrypoint.sh /usr/local/bin/hive-brain

USER hive
ENV PATH=/home/hive/.hive/bin:$PATH \
    HIVE_HOME=/data/hive \
    HIVE_HUB_PORT=8765
VOLUME ["/data"]
EXPOSE 8765
HEALTHCHECK --interval=30s --timeout=5s --start-period=60s --retries=3 \
  CMD curl -fsS "http://127.0.0.1:${HIVE_HUB_PORT}/api/v1/health" >/dev/null || exit 1
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/hive-brain"]
