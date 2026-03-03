# Tipos de Serviço Mistos

A maioria dos projetos se encaixa perfeitamente em um modelo: um `docker-compose.yml` que orquestra tudo, ou `[services]` que executam processos simples. Alguns projetos precisam dos dois ao mesmo tempo. O Coast oferece suporte à definição de `compose` e `[services]` no mesmo Coastfile para que o Docker Compose gerencie seus serviços pesados e com estado, enquanto processos simples lidam com ferramentas leves que pertencem ao host DinD.

## Quando Você Precisa de Serviços Mistos

O cenário mais comum é um grande monorepo em que a pilha principal da aplicação (servidores web, bancos de dados, workers em segundo plano) já está conteinerizada via Docker Compose, mas certas ferramentas de desenvolvimento funcionam melhor como processos no nível do host:

- **Servidores de desenvolvimento Vite / Webpack** que precisam ser acessíveis de dentro de contêineres do compose via `host.docker.internal`. Executá-los como serviços simples no host DinD evita gambiarras de rede.
- **Observadores de arquivos ou geradores de código** que precisam de acesso rápido ao sistema de arquivos no `/workspace` montado via bind mount sem passar pelo sistema de arquivos overlay de um contêiner interno.
- **Language servers ou ferramentas de build** que são mais simples de executar como um processo do que de incorporar em uma imagem de contêiner.

Se seus serviços do compose precisam alcançar um processo no nível do host por nome de host, serviços mistos são a resposta certa.

## Configuração

Defina `compose` para sua pilha Docker Compose e adicione seções `[services.*]` para processos simples no mesmo Coastfile:

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

A seção `[coast.setup]` instala os pacotes necessários para serviços simples. Serviços do compose obtêm seus runtimes de seus Dockerfiles como de costume.

## Como Funciona

No `coast run`, o Coast inicia ambos os tipos de serviço sequencialmente:

1. Serviços do compose iniciam primeiro via `docker compose up -d` dentro do contêiner DinD e o Coast aguarda as verificações de saúde passarem
2. Serviços simples iniciam em seguida via os scripts do supervisor em `/coast-supervisor/`

Ambos os tipos rodam concorrentemente após iniciarem. Serviços do compose rodam como contêineres internos gerenciados pelo daemon DinD. Serviços simples rodam como processos comuns no SO do host DinD.

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

## Rede Entre Tipos de Serviço

Serviços do compose rodam dentro da rede do daemon Docker interno. Serviços simples rodam no host DinD. Para permitir que contêineres do compose alcancem um serviço simples:

- Serviços simples devem fazer bind em `0.0.0.0` para que sejam acessíveis de dentro da rede Docker
- Contêineres do compose podem acessar serviços simples via `host.docker.internal` (disponível automaticamente em contêineres DinD)
- Defina variáveis de ambiente no seu arquivo compose para apontar para o serviço simples: `VITE_HOST=host.docker.internal:3040`

Serviços simples podem acessar contêineres do compose via `localhost:<published-port>` já que as portas do compose são publicadas no host DinD.

## Comandos

Todos os comandos do Coast funcionam com ambos os tipos de serviço de forma transparente:

| Command | Compose services | Bare services |
|---|---|---|
| `coast ps` | Mostra com `kind: compose` | Mostra com `kind: bare` |
| `coast logs` | Obtém via `docker compose logs` | Acompanha (tail) de `/var/log/coast-services/` |
| `coast logs --service <name>` | Direciona para compose se for serviço compose | Direciona para bare se for serviço bare |
| `coast restart-services` | Executa compose down + up | Executa stop-all + start-all |
| `coast assign` | Lida com rebuild/restart do compose | Reexecuta comandos de instalação e reinicia |
| `coast stop` / `coast start` | Para/inicia o compose | Para/inicia serviços simples |

Ao solicitar logs de um serviço específico, o Coast determina automaticamente o tipo de serviço e direciona para a fonte de logs correta.

## Troca de Branch

No `coast assign`, ambos os tipos de serviço são tratados:

1. Serviços do compose são derrubados (torn down) ou recriados à força dependendo da estratégia de assign (`restart`, `rebuild`, `hot`)
2. Serviços simples são parados, comandos de instalação são reexecutados e os serviços reiniciam

A seção `[assign.services]` aceita tanto nomes de serviços do compose quanto nomes de serviços simples. Use as mesmas estratégias para ambos:

```toml
[assign.services]
web = "rebuild"          # compose service — rebuild image on branch switch
sidekiq = "restart"      # compose service — restart container
vite-web = "restart"     # bare service — stop, re-install, restart
```

## Serviços Compartilhados

[Serviços compartilhados](SHARED_SERVICES.md) funcionam de forma independente tanto de serviços do compose quanto de serviços simples. Eles rodam no daemon Docker do host e são acessíveis de dentro do Coast independentemente de quais tipos de serviço estejam configurados.

## Migração de Apenas Serviços Simples para Misto

Se você começou com um Coastfile apenas com serviços simples e quer conteinerizar alguns serviços mantendo outros como processos simples:

1. Escreva Dockerfiles e um `docker-compose.yml` para os serviços que você quer conteinerizar
2. Adicione `compose = "./docker-compose.yml"` à seção `[coast]`
3. Remova as seções `[services.*]` para serviços que agora estão no compose
4. Mantenha `[services.*]` para processos que devem continuar simples
5. Faça rebuild com `coast build`
