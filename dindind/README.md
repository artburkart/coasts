# dindind

Docker-in-Docker integration test environment and install script test suite.

These tests can only be run on macOS (Apple Silicon).

## Tests

### test_install

Tests the Coast install script (`external/install.sh`) on both Linux and macOS.

```bash
./dindind/tests/test_install.sh linux    # Ubuntu DinD container
./dindind/tests/test_install.sh macos    # Tart macOS VM
```

Assertions:

- Binaries land in `~/.coast/bin/`
- PATH gets added to the correct shell rc file
- `coast` and `coastd` are co-located
- Stale binaries at other PATH locations are detected
- `eval` vs `| sh` invocation behavior
- Docker-not-installed warning fires when Docker is absent

### Dependencies

**Linux target** requires Docker only (uses the existing `dindind/lib/base.Dockerfile`).

**macOS target** requires two additional tools:

#### tart

macOS VM runner using Apple Virtualization.framework.

```bash
brew install cirruslabs/cli/tart
```

If Homebrew fails (Xcode version mismatch), install from the GitHub release:

```bash
cd /tmp
curl -fsSL "https://github.com/cirruslabs/tart/releases/latest/download/tart.tar.gz" -o tart.tar.gz
tar xzf tart.tar.gz
mkdir -p ~/.local/share ~/.local/bin
cp -R tart.app ~/.local/share/tart.app
ln -sf ~/.local/share/tart.app/Contents/MacOS/tart ~/.local/bin/tart
```

Make sure `~/.local/bin` is on your PATH.

#### sshpass

Used for SSH password auth into the Tart VM.

```bash
brew install esolitos/ipa/sshpass
```

If Homebrew fails, build from source:

```bash
cd /tmp
curl -fsSL "https://sourceforge.net/projects/sshpass/files/sshpass/1.10/sshpass-1.10.tar.gz/download" -o sshpass-1.10.tar.gz
tar xzf sshpass-1.10.tar.gz
cd sshpass-1.10
./configure --prefix="$HOME/.local"
make && make install
```

### First run

The macOS target pulls a ~24 GB VM image on first run. Subsequent runs reuse the cached image and clone from it instantly.

## Scenarios

The `scenarios/` directory contains issue-specific reproduction environments (e.g., `wsl-ubuntu/` for issue #130). These are separate from the `tests/` directory and are run via `dindind/run.sh`.
