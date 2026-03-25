#!/usr/bin/env bash
# Scenario-specific environment overrides for WSL Ubuntu simulation.
# Sourced by test scripts before running checks.

export WSL_DISTRO_NAME="${WSL_DISTRO_NAME:-Ubuntu}"
export WSL_INTEROP="${WSL_INTEROP:-/run/WSL/1_interop}"
