# WSL2 Ubuntu simulation for reproducing issue #130.
# Extends the shared base image with WSL environment variables so that
# Coast's WSL detection (env vars + /proc/version) triggers.
FROM coast-dindind-base AS base

# Simulate WSL2 environment variables
ENV WSL_DISTRO_NAME=Ubuntu
ENV WSL_INTEROP=/run/WSL/1_interop
ENV SHELL=/bin/bash

# Copy scenario test scripts
COPY scenarios/wsl-ubuntu/test-install.sh /test-scripts/test-install.sh
COPY scenarios/wsl-ubuntu/env.sh          /test-scripts/env.sh
COPY lib/test-local-build.sh              /test-scripts/test-local-build.sh
RUN chmod +x /test-scripts/*.sh

USER testuser
WORKDIR /home/testuser
