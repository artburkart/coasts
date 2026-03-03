# 혼합 서비스 유형

대부분의 프로젝트는 하나의 모델에 깔끔하게 들어맞습니다. 모든 것을 오케스트레이션하는 `docker-compose.yml`을 쓰거나, 일반 프로세스로 실행되는 `[services]`를 쓰는 방식입니다. 일부 프로젝트는 두 가지가 동시에 필요합니다. Coast는 동일한 Coastfile에서 `compose`와 `[services]`를 함께 정의하는 것을 지원하여, Docker Compose가 무겁고 상태를 가지는 서비스들을 관리하고, 단순 프로세스는 DinD 호스트에 두는 것이 적합한 가벼운 도구들을 처리할 수 있게 합니다.

## 혼합 서비스가 필요한 경우

가장 흔한 시나리오는 큰 모노레포로, 핵심 애플리케이션 스택(웹 서버, 데이터베이스, 백그라운드 워커)은 이미 Docker Compose로 컨테이너화되어 있지만, 특정 개발 도구는 호스트 수준 프로세스로 실행하는 편이 더 나은 경우입니다:

- `host.docker.internal`을 통해 compose 컨테이너 내부에서 접근 가능해야 하는 **Vite / Webpack 개발 서버**. DinD 호스트에서 베어 서비스로 실행하면 네트워킹 꼼수를 피할 수 있습니다.
- 내부 컨테이너의 오버레이 파일시스템을 거치지 않고, 바인드 마운트된 `/workspace`에 빠르게 접근해야 하는 **파일 워처 또는 코드 생성기**.
- 컨테이너 이미지에 굽기보다 프로세스로 실행하는 편이 더 단순한 **언어 서버 또는 빌드 도구**.

compose 서비스가 호스트 수준 프로세스에 호스트명으로 접근해야 한다면, 혼합 서비스가 정답입니다.

## 구성

Docker Compose 스택을 위해 `compose`를 설정하고, 동일한 Coastfile에 베어 프로세스를 위한 `[services.*]` 섹션을 추가하세요:

```toml
[coast]
name = "my-app"
compose = "./docker-compose.yml"

[coast.setup]
packages = ["nodejs", "npm"]

# Bare services run directly on the DinD host
[services.vite-web]
install = "cd /workspace && npm install"
command = "cd /workspace && npm run dev"
port = 3040
restart = "on-failure"

# Compose services are defined in docker-compose.yml as usual
# (web, postgres, redis, sidekiq, etc.)

[ports]
web = 3000
vite-web = 3040
postgres = 5432

[assign]
default = "none"

[assign.services]
web = "restart"
sidekiq = "restart"
vite-web = "restart"
```

`[coast.setup]` 섹션은 베어 서비스에 필요한 패키지를 설치합니다. compose 서비스는 평소처럼 Dockerfile에서 런타임을 가져옵니다.

## 동작 방식

`coast run` 시 Coast는 두 서비스 유형을 순차적으로 시작합니다:

1. Compose 서비스가 DinD 컨테이너 내부에서 `docker compose up -d`로 먼저 시작되며, Coast는 헬스 체크가 통과할 때까지 기다립니다
2. 다음으로 베어 서비스가 `/coast-supervisor/`의 supervisor 스크립트를 통해 시작됩니다

시작된 이후에는 두 유형이 동시에 실행됩니다. compose 서비스는 DinD 데몬이 관리하는 내부 컨테이너로 실행됩니다. 베어 서비스는 DinD 호스트 OS에서 일반 프로세스로 실행됩니다.

```text
┌─── Coast: dev-1 ──────────────────────────────────────────┐
│                                                            │
│   Inner Docker daemon (compose services)                   │
│   ├── web           (Rails, :3000)                         │
│   ├── postgres      (database, :5432)                      │
│   └── redis         (cache, :6379)                         │
│                                                            │
│   /coast-supervisor/ (bare services)                       │
│   ├── vite-web.sh   (node process, :3040)                  │
│   └── vite-admin.sh (node process, :3041)                  │
│                                                            │
│   /workspace ← bind mount of project root                  │
└────────────────────────────────────────────────────────────┘
```

## 서비스 유형 간 네트워킹

Compose 서비스는 내부 Docker 데몬의 네트워크 안에서 실행됩니다. 베어 서비스는 DinD 호스트에서 실행됩니다. compose 컨테이너가 베어 서비스에 접근할 수 있게 하려면:

- 베어 서비스는 Docker 네트워크 내부에서 접근 가능하도록 `0.0.0.0`에 바인딩해야 합니다
- Compose 컨테이너는 `host.docker.internal`을 통해 베어 서비스에 접근할 수 있습니다(DinD 컨테이너에서 자동으로 사용 가능)
- compose 파일에서 베어 서비스를 가리키도록 환경 변수를 설정하세요: `VITE_HOST=host.docker.internal:3040`

베어 서비스는 compose 포트가 DinD 호스트에 퍼블리시되므로 `localhost:<published-port>`를 통해 compose 컨테이너에 접근할 수 있습니다.

## 명령어

모든 Coast 명령은 두 서비스 유형 모두에서 투명하게 동작합니다:

| Command | Compose services | Bare services |
|---|---|---|
| `coast ps` | `kind: compose`로 표시 | `kind: bare`로 표시 |
| `coast logs` | `docker compose logs`로 가져옴 | `/var/log/coast-services/`에서 tail |
| `coast logs --service <name>` | compose 서비스면 compose로 라우팅 | bare 서비스면 bare로 라우팅 |
| `coast restart-services` | compose down + up 실행 | stop-all + start-all 실행 |
| `coast assign` | compose 재빌드/재시작 처리 | install 명령을 다시 실행하고 재시작 |
| `coast stop` / `coast start` | compose 중지/시작 | bare 서비스 중지/시작 |

특정 서비스의 로그를 요청하면, Coast는 서비스 유형을 자동으로 판별해 올바른 로그 소스로 라우팅합니다.

## 브랜치 전환

`coast assign` 시 두 서비스 유형 모두가 처리됩니다:

1. Compose 서비스는 assign 전략(`restart`, `rebuild`, `hot`)에 따라 내려가거나 강제로 재생성됩니다
2. 베어 서비스는 중지되고, install 명령이 다시 실행된 뒤 서비스가 재시작됩니다

`[assign.services]` 섹션은 compose 서비스 이름과 베어 서비스 이름을 모두 받을 수 있습니다. 둘 다 같은 전략을 사용하세요:

```toml
[assign.services]
web = "rebuild"          # compose service — rebuild image on branch switch
sidekiq = "restart"      # compose service — restart container
vite-web = "restart"     # bare service — stop, re-install, restart
```

## 공유 서비스

[공유 서비스](SHARED_SERVICES.md)는 compose 및 베어 서비스와 독립적으로 동작합니다. 이들은 호스트 Docker 데몬에서 실행되며, 어떤 서비스 유형이 구성되어 있든 Coast 내부에서 접근 가능합니다.

## 베어 전용에서 혼합으로 마이그레이션

베어 서비스만 있는 Coastfile로 시작했는데 일부 서비스를 컨테이너화하면서 다른 서비스는 베어 프로세스로 유지하고 싶다면:

1. 컨테이너화할 서비스들에 대해 Dockerfile과 `docker-compose.yml`을 작성합니다
2. `[coast]` 섹션에 `compose = "./docker-compose.yml"`을 추가합니다
3. 이제 compose에 포함된 서비스에 대한 `[services.*]` 섹션을 제거합니다
4. 베어로 남겨야 하는 프로세스에 대해서는 `[services.*]`를 유지합니다
5. `coast build`로 다시 빌드합니다
