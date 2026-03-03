# Смешанные типы сервисов

Большинство проектов аккуратно укладываются в одну модель: `docker-compose.yml`, который оркестрирует всё, или `[services]`, которые запускают обычные процессы. Некоторым проектам нужно и то и другое одновременно. Coast поддерживает определение `compose` и `[services]` в одном Coastfile, чтобы Docker Compose управлял вашими тяжёлыми, состояниями обладающими сервисами, а «голые» процессы обрабатывали лёгкие инструменты, которым место на DinD-хосте.

## Когда нужны смешанные сервисы

Самый распространённый сценарий — большой монорепозиторий, где основной стек приложения (веб‑серверы, базы данных, фоновые воркеры) уже контейнеризован через Docker Compose, но некоторые инструменты разработки лучше работают как процессы на уровне хоста:

- **Dev-серверы Vite / Webpack**, которые должны быть доступны изнутри compose-контейнеров через `host.docker.internal`. Запуск их как bare-сервисов на DinD-хосте позволяет избежать сетевых хаков.
- **Наблюдатели за файлами или генераторы кода**, которым нужен быстрый доступ к файловой системе bind-mounted `/workspace` без прохождения через overlay-файловую систему внутреннего контейнера.
- **Языковые серверы или инструменты сборки**, которые проще запустить как процесс, чем запекать в образ контейнера.

Если вашим compose-сервисам нужно обращаться к процессу на уровне хоста по имени хоста, смешанные сервисы — правильный ответ.

## Конфигурация

Задайте `compose` для вашего стека Docker Compose и добавьте секции `[services.*]` для «голых» процессов в том же Coastfile:

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

Секция `[coast.setup]` устанавливает пакеты, необходимые для bare-сервисов. Compose-сервисы, как обычно, получают свои рантаймы из Dockerfile.

## Как это работает

При `coast run` Coast запускает оба типа сервисов последовательно:

1. Сначала запускаются compose-сервисы через `docker compose up -d` внутри DinD-контейнера, и Coast ждёт прохождения health checks
2. Затем запускаются bare-сервисы через скрипты супервизора в `/coast-supervisor/`

После запуска оба типа работают параллельно. Compose-сервисы работают как внутренние контейнеры, управляемые демоном DinD. Bare-сервисы работают как обычные процессы в ОС DinD-хоста.

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

## Сеть между типами сервисов

Compose-сервисы работают внутри сети внутреннего демона Docker. Bare-сервисы работают на DinD-хосте. Чтобы compose-контейнеры могли обращаться к bare-сервису:

- Bare-сервисы должны слушать на `0.0.0.0`, чтобы быть доступными изнутри Docker-сети
- Compose-контейнеры могут обращаться к bare-сервисам через `host.docker.internal` (автоматически доступно в DinD-контейнерах)
- Задайте переменные окружения в вашем compose-файле, чтобы указывать на bare-сервис: `VITE_HOST=host.docker.internal:3040`

Bare-сервисы могут обращаться к compose-контейнерам через `localhost:<published-port>`, поскольку compose-порты опубликованы на DinD-хосте.

## Команды

Все команды Coast прозрачно работают с обоими типами сервисов:

| Command | Compose services | Bare services |
|---|---|---|
| `coast ps` | Показывает с `kind: compose` | Показывает с `kind: bare` |
| `coast logs` | Получает через `docker compose logs` | «Хвостит» из `/var/log/coast-services/` |
| `coast logs --service <name>` | Направляет в compose, если сервис compose | Направляет в bare, если сервис bare |
| `coast restart-services` | Выполняет compose down + up | Выполняет stop-all + start-all |
| `coast assign` | Обрабатывает rebuild/restart compose | Повторно запускает install-команды и перезапускает |
| `coast stop` / `coast start` | Останавливает/запускает compose | Останавливает/запускает bare-сервисы |

При запросе логов для конкретного сервиса Coast автоматически определяет тип сервиса и направляет к правильному источнику логов.

## Переключение веток

При `coast assign` обрабатываются оба типа сервисов:

1. Compose-сервисы останавливаются или принудительно пересоздаются в зависимости от стратегии assign (`restart`, `rebuild`, `hot`)
2. Bare-сервисы останавливаются, команды install выполняются заново, и сервисы перезапускаются

Секция `[assign.services]` принимает как имена compose-сервисов, так и имена bare-сервисов. Используйте одинаковые стратегии для обоих:

```toml
[assign.services]
web = "rebuild"          # compose service — rebuild image on branch switch
sidekiq = "restart"      # compose service — restart container
vite-web = "restart"     # bare service — stop, re-install, restart
```

## Общие сервисы

[Общие сервисы](SHARED_SERVICES.md) работают независимо и от compose, и от bare-сервисов. Они запускаются на хостовом демоне Docker и доступны изнутри Coast независимо от того, какие типы сервисов настроены.

## Миграция с Bare-Only на смешанный режим

Если вы начали с Coastfile, где были только bare-сервисы, и хотите контейнеризовать некоторые сервисы, оставив другие как bare-процессы:

1. Напишите Dockerfile и `docker-compose.yml` для сервисов, которые вы хотите контейнеризовать
2. Добавьте `compose = "./docker-compose.yml"` в секцию `[coast]`
3. Удалите секции `[services.*]` для сервисов, которые теперь находятся в compose
4. Оставьте `[services.*]` для процессов, которые должны остаться bare
5. Пересоберите с `coast build`
