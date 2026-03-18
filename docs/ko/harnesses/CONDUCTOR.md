# Conductor

[Conductor](https://conductor.build/)는 병렬 Claude Code 에이전트를 실행하며, 각 에이전트는 자체적으로 격리된 워크스페이스를 가집니다. 워크스페이스는 `~/conductor/workspaces/<project-name>/`에 저장된 git worktree입니다. 각 워크스페이스는 이름이 지정된 브랜치로 체크아웃됩니다.

이러한 worktree는 프로젝트 루트 외부에 존재하므로, Coast가 이를 발견하고 마운트하려면 명시적인 구성이 필요합니다.

## 설정

`~/conductor/workspaces/<project-name>`를 `worktree_dir`에 추가하세요. Codex와 달리(모든 프로젝트를 하나의 평면 디렉터리 아래에 저장함), Conductor는 프로젝트별 하위 디렉터리 아래에 worktree를 중첩하므로 경로에 프로젝트 이름이 포함되어야 합니다:

```toml
[coast]
name = "my-app"
worktree_dir = [".worktrees", "~/conductor/workspaces/my-app"]
```

Conductor는 저장소별로 워크스페이스 경로를 구성할 수 있으므로, 기본값인 `~/conductor/workspaces`가 현재 설정과 일치하지 않을 수 있습니다. 실제 경로를 확인하려면 Conductor 저장소 설정을 확인하고 그에 맞게 조정하세요 — 디렉터리가 어디에 있든 원리는 동일합니다.

Coast는 런타임에 `~`를 확장하며, `~/` 또는 `/`로 시작하는 모든 경로를 외부 경로로 취급합니다. 자세한 내용은 [Worktree Directories](../coastfiles/WORKTREE_DIR.md)를 참조하세요.

`worktree_dir`를 변경한 후에는 bind mount가 적용되도록 기존 인스턴스를 **다시 생성**해야 합니다:

```bash
coast rm my-instance
coast build
coast run my-instance
```

worktree 목록은 즉시 업데이트됩니다(Coast가 새 Coastfile을 읽기 때문입니다). 그러나 Conductor worktree에 할당하려면 컨테이너 내부의 bind mount가 필요합니다.

## Coast가 수행하는 작업

- **Bind mount** — 컨테이너 생성 시 Coast는 `~/conductor/workspaces/<project-name>`를 컨테이너의 `/host-external-wt/{index}`에 마운트합니다.
- **Discovery** — `git worktree list --porcelain`는 저장소 범위로 동작하므로 현재 프로젝트에 속한 worktree만 표시됩니다.
- **Naming** — Conductor worktree는 이름이 지정된 브랜치를 사용하므로 Coast UI와 CLI에 브랜치 이름으로 표시됩니다(예: `scroll-to-bottom-btn`). 하나의 브랜치는 한 번에 하나의 Conductor 워크스페이스에서만 체크아웃할 수 있습니다.
- **Assign** — `coast assign`은 외부 bind mount 경로에서 `/workspace`를 다시 마운트합니다.
- **Gitignored sync** — 호스트 파일시스템에서 절대 경로로 실행되며, bind mount 없이도 동작합니다.
- **Orphan detection** — git watcher는 외부 디렉터리를 재귀적으로 스캔하고 `.git` gitdir 포인터를 기준으로 필터링합니다. Conductor가 워크스페이스를 보관 처리하거나 삭제하면 Coast는 인스턴스 할당을 자동으로 해제합니다.

## 예시

```toml
[coast]
name = "my-app"
compose = "./docker-compose.yml"
worktree_dir = [".worktrees", ".claude/worktrees", "~/.codex/worktrees", "~/conductor/workspaces/my-app"]
primary_port = "web"

[ports]
web = 3000
api = 8080

[assign]
default = "none"
[assign.services]
web = "hot"
api = "hot"
```

- `.worktrees/` — Coast가 관리하는 worktree
- `.claude/worktrees/` — Claude Code(로컬, 특별한 처리 없음)
- `~/.codex/worktrees/` — Codex(외부, bind-mounted)
- `~/conductor/workspaces/my-app/` — Conductor(외부, bind-mounted)

## Conductor 환경 변수

- Coast 내부 런타임 구성에 Conductor 전용 환경 변수(예: `CONDUCTOR_PORT`, `CONDUCTOR_WORKSPACE_PATH`)에 의존하지 마세요. Coast는 포트, 워크스페이스 경로, 서비스 검색을 독립적으로 관리합니다 — 대신 Coastfile의 `[ports]`와 `coast exec`를 사용하세요.
