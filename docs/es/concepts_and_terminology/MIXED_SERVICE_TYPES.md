# Tipos de Servicios Mixtos

La mayoría de los proyectos encajan perfectamente en un modelo: un `docker-compose.yml` que orquesta todo, o `[services]` que ejecutan procesos simples. Algunos proyectos necesitan ambos al mismo tiempo. Coast admite definir `compose` y `[services]` en el mismo Coastfile para que Docker Compose administre tus servicios pesados y con estado, mientras que los procesos sin contenedor manejan herramientas ligeras que pertenecen al host de DinD.

## Cuándo Necesitas Servicios Mixtos

El escenario más común es un monorepo grande donde el stack principal de la aplicación (servidores web, bases de datos, workers en segundo plano) ya está contenerizado mediante Docker Compose, pero ciertas herramientas de desarrollo funcionan mejor como procesos a nivel de host:

- **Servidores de desarrollo Vite / Webpack** que necesitan ser accesibles desde dentro de contenedores de compose a través de `host.docker.internal`. Ejecutarlos como servicios sin contenedor en el host de DinD evita trucos de red.
- **Watchers de archivos o generadores de código** que necesitan acceso rápido al sistema de archivos del bind-mounted `/workspace` sin pasar por el sistema de archivos overlay de un contenedor interno.
- **Servidores de lenguaje o herramientas de build** que es más simple ejecutar como un proceso que incorporarlas en una imagen de contenedor.

Si tus servicios de compose necesitan llegar a un proceso a nivel de host por nombre de host, los servicios mixtos son la respuesta correcta.

## Configuración

Configura `compose` para tu stack de Docker Compose y agrega secciones `[services.*]` para procesos sin contenedor en el mismo Coastfile:

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

La sección `[coast.setup]` instala paquetes necesarios para los servicios sin contenedor. Los servicios de compose obtienen sus runtimes de sus Dockerfiles como siempre.

## Cómo Funciona

En `coast run`, Coast inicia ambos tipos de servicios de forma secuencial:

1. Los servicios de compose se inician primero mediante `docker compose up -d` dentro del contenedor DinD y Coast espera a que pasen los health checks
2. Luego se inician los servicios sin contenedor mediante los scripts del supervisor en `/coast-supervisor/`

Ambos tipos se ejecutan de forma concurrente una vez iniciados. Los servicios de compose se ejecutan como contenedores internos administrados por el daemon de DinD. Los servicios sin contenedor se ejecutan como procesos simples en el sistema operativo host de DinD.

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

## Redes Entre Tipos de Servicios

Los servicios de compose se ejecutan dentro de la red del daemon interno de Docker. Los servicios sin contenedor se ejecutan en el host de DinD. Para permitir que los contenedores de compose alcancen un servicio sin contenedor:

- Los servicios sin contenedor deberían hacer bind a `0.0.0.0` para que sean accesibles desde dentro de la red de Docker
- Los contenedores de compose pueden alcanzar los servicios sin contenedor mediante `host.docker.internal` (disponible automáticamente en contenedores DinD)
- Configura variables de entorno en tu archivo compose para apuntar al servicio sin contenedor: `VITE_HOST=host.docker.internal:3040`

Los servicios sin contenedor pueden alcanzar los contenedores de compose mediante `localhost:<published-port>` ya que los puertos de compose se publican en el host de DinD.

## Comandos

Todos los comandos de Coast funcionan con ambos tipos de servicios de forma transparente:

| Command | Compose services | Bare services |
|---|---|---|
| `coast ps` | Muestra con `kind: compose` | Muestra con `kind: bare` |
| `coast logs` | Obtiene mediante `docker compose logs` | Hace tail desde `/var/log/coast-services/` |
| `coast logs --service <name>` | Enruta a compose si es servicio compose | Enruta a bare si es servicio bare |
| `coast restart-services` | Ejecuta compose down + up | Ejecuta stop-all + start-all |
| `coast assign` | Maneja rebuild/restart de compose | Re-ejecuta comandos de instalación y reinicia |
| `coast stop` / `coast start` | Detiene/inicia compose | Detiene/inicia servicios sin contenedor |

Al solicitar logs para un servicio específico, Coast determina el tipo de servicio automáticamente y enruta a la fuente de logs correcta.

## Cambio de Rama

En `coast assign`, se manejan ambos tipos de servicios:

1. Los servicios de compose se desmantelan o se fuerzan a recrearse dependiendo de la estrategia de assign (`restart`, `rebuild`, `hot`)
2. Los servicios sin contenedor se detienen, se vuelven a ejecutar los comandos de instalación y los servicios se reinician

La sección `[assign.services]` acepta tanto nombres de servicios de compose como nombres de servicios sin contenedor. Usa las mismas estrategias para ambos:

```toml
[assign.services]
web = "rebuild"          # compose service — rebuild image on branch switch
sidekiq = "restart"      # compose service — restart container
vite-web = "restart"     # bare service — stop, re-install, restart
```

## Servicios Compartidos

Los [servicios compartidos](SHARED_SERVICES.md) funcionan de manera independiente tanto de los servicios de compose como de los servicios sin contenedor. Se ejecutan en el daemon de Docker del host y son accesibles desde dentro de Coast independientemente de qué tipos de servicios estén configurados.

## Migrar de Solo Bare a Mixto

Si empezaste con un Coastfile solo de servicios sin contenedor y quieres contenerizar algunos servicios manteniendo otros como procesos sin contenedor:

1. Escribe Dockerfiles y un `docker-compose.yml` para los servicios que quieres contenerizar
2. Agrega `compose = "./docker-compose.yml"` a la sección `[coast]`
3. Elimina las secciones `[services.*]` para los servicios que ahora están en compose
4. Mantén `[services.*]` para procesos que deben seguir siendo sin contenedor
5. Reconstruye con `coast build`
