# 混合服务类型

大多数项目都能很好地契合一种模型:使用 `docker-compose.yml` 来编排一切，或使用运行普通进程的 `[services]`。有些项目需要同时使用两者。Coast 支持在同一个 Coastfile 中同时定义 `compose` 和 `[services]`，让 Docker Compose 管理重量级、有状态的服务，同时让裸进程处理那些适合运行在 DinD 主机上的轻量级工具。

## 何时需要混合服务

最常见的场景是一个大型 monorepo:核心应用栈（Web 服务器、数据库、后台 worker）已经通过 Docker Compose 容器化，但某些开发工具以主机级进程运行效果更好:

- 需要从 compose 容器内部通过 `host.docker.internal` 访问的 **Vite / Webpack 开发服务器**。将它们作为 DinD 主机上的裸服务运行可以避免网络方面的黑魔法。
- 需要对 bind-mounted 的 `/workspace` 进行快速文件系统访问的 **文件监视器或代码生成器**，而无需经过内部容器的 overlay 文件系统。
- 作为进程运行更简单、无需烘焙进容器镜像的 **语言服务器或构建工具**。

如果你的 compose 服务需要通过主机名访问某个主机级进程，混合服务就是正确答案。

## 配置

为你的 Docker Compose 栈设置 `compose`，并在同一个 Coastfile 中添加用于裸进程的 `[services.*]` 小节:

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

`[coast.setup]` 小节会安装裸服务所需的包。Compose 服务则一如既往地从它们的 Dockerfile 获取运行时环境。

## 工作原理

执行 `coast run` 时，Coast 会按顺序启动两种服务:

1. Compose 服务先通过 DinD 容器内的 `docker compose up -d` 启动，并且 Coast 会等待健康检查通过
2. 接着裸服务通过 `/coast-supervisor/` 中的 supervisor 脚本启动

两者一旦启动就会并发运行。Compose 服务作为由 DinD daemon 管理的内部容器运行。裸服务则作为普通进程运行在 DinD 主机操作系统上。

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

## 服务类型之间的网络通信

Compose 服务运行在内部 Docker daemon 的网络中。裸服务运行在 DinD 主机上。要让 compose 容器访问裸服务:

- 裸服务应绑定到 `0.0.0.0`，以便从 Docker 网络内部可达
- Compose 容器可以通过 `host.docker.internal` 访问裸服务（在 DinD 容器中会自动可用）
- 在你的 compose 文件中设置环境变量来指向裸服务:`VITE_HOST=host.docker.internal:3040`

裸服务可以通过 `localhost:<published-port>` 访问 compose 容器，因为 compose 端口会发布到 DinD 主机上。

## 命令

所有 Coast 命令都能透明地同时适用于两种服务类型:

| Command | Compose services | Bare services |
|---|---|---|
| `coast ps` | 显示为 `kind: compose` | 显示为 `kind: bare` |
| `coast logs` | 通过 `docker compose logs` 获取 | 从 `/var/log/coast-services/` 进行 tail |
| `coast logs --service <name>` | 若为 compose 服务则路由到 compose | 若为裸服务则路由到裸服务 |
| `coast restart-services` | 执行 compose down + up | 执行 stop-all + start-all |
| `coast assign` | 处理 compose 的 rebuild/restart | 重新运行 install 命令并重启 |
| `coast stop` / `coast start` | 停止/启动 compose | 停止/启动裸服务 |

当请求某个特定服务的日志时，Coast 会自动判断服务类型并路由到正确的日志来源。

## 分支切换

执行 `coast assign` 时，会同时处理两种服务类型:

1. Compose 服务会根据 assign 策略（`restart`、`rebuild`、`hot`）被拆除或强制重新创建
2. 裸服务会被停止、重新运行 install 命令，然后服务重启

`[assign.services]` 小节同时接受 compose 服务名和裸服务名。两者使用相同的策略:

```toml
[assign.services]
web = "rebuild"          # compose service — rebuild image on branch switch
sidekiq = "restart"      # compose service — restart container
vite-web = "restart"     # bare service — stop, re-install, restart
```

## 共享服务

[共享服务](SHARED_SERVICES.md) 独立于 compose 与裸服务工作。它们运行在宿主机 Docker daemon 上，并且无论配置了哪种服务类型，都可以从 Coast 内部访问。

## 从仅裸服务迁移到混合模式

如果你一开始使用仅裸服务的 Coastfile，后来想把一部分服务容器化，同时保留其他服务为裸进程:

1. 为你想要容器化的服务编写 Dockerfile 和 `docker-compose.yml`
2. 在 `[coast]` 小节中添加 `compose = "./docker-compose.yml"`
3. 删除那些现在已经在 compose 中的服务对应的 `[services.*]` 小节
4. 保留应继续作为裸进程的 `[services.*]`
5. 使用 `coast build` 重新构建
