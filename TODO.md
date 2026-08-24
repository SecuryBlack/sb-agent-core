# sb-agent-core — TODO / Definición

> **Estado:** solo diseño. No hay código. Recoge la sesión del 2026-08-21.
> Objetivo: dejar de copiar-pegar entre los agentes Rust de SecuryBlack sin renunciar
> a que cada uno sea su propio repo open source.

Afecta a: `oxi-pulse`, `ferro-sentry`, `cupra-flow`, `nexus-agent` y el futuro `cromo-forge`.

---

## El problema, medido

`updater/mod.rs` es copy-paste literal entre repos:

- **OxiPulse vs FerroSentry:** 58 líneas idénticas salvo 2 constantes (`GITHUB_REPO`, `bin_name`).
- **OxiPulse vs Nexus:** las mismas 2 constantes, **más `STARTUP_DELAY` cambiado de 60s a 300s**
  y los comentarios reescritos.

Ese último caso ya no es duplicación, es **drift**: dos comportamientos distintos que nadie
decidió conscientemente. Es la señal de que el problema es real y no teórico.

Resto de duplicación:

| Duplicado | Dónde | Tamaño |
|---|---|---|
| Carga de config (TOML + env + rutas por SO) | 4 implementaciones separadas | 143–235 líneas cada una |
| Wrapper de servicio Windows (`windows-service`) | los 4 | dentro de `main.rs` |
| Init de logging (`tracing` + `tracing-appender`) | los 4 | idéntico |
| `install.sh` / `install.ps1` | oxi, ferro, nexus | 169–371 líneas, mismo esqueleto |
| `proto/tunnel/v1/tunnel.proto` | nexus **y** ferro | el mismo fichero en dos repos |

**Matiz verificado sobre el `.proto`:** hoy las dos copias solo difieren en comentarios, no hay
drift semántico *todavía*. Pero es el que más riesgo tiene: el día que Nexus cambie un campo y
FerroSentry no se entere, falla en producción del cliente y el error no dirá nada útil.

---

## Decisión: crates publicados, no monorepo

| Opción | Veredicto |
|---|---|
| **Monorepo** (workspace único) | **No.** Rompe lo ya construido: OxiPulse tiene badges, releases e identidad propia. El refactor atómico no compensa perder cuatro proyectos OSS con cara. |
| **Submodules / subtree** | **No.** Barato de montar, malo de vivir. Versionado confuso. |
| **Seguir copiando** | Es el statu quo, y ya se rompió (el `STARTUP_DELAY`). Con CromoForge serían cinco copias. |
| **Crates publicados** | **Sí.** Lo idiomático en Rust y lo único compatible con repos separados. |

### Dos crates, no uno

**`sb-agent-core`** — runtime común, cero dominio:

- Carga de config (TOML + env + rutas por SO). Base: el de OxiPulse, que es el más completo (235 líneas).
- Init de logging con rotación (`tracing` + `tracing-appender`).
- Wrapper de servicio: systemd + Windows SCM, y manejo de señales.
- Updater desde GitHub Releases, **parametrizado** por repo y `bin_name` en vez de copy-paste.
- Buffer offline con backoff exponencial (hoy solo en OxiPulse; lo van a querer todos).
- **Status socket** (ver sección TUI) — la pieza más importante del crate.

**`sb-conduit`** — el Conduit Protocol: el `.proto` y sus stubs generados **una sola vez**.
Consumido por Nexus, FerroSentry y CromoForge. Elimina el riesgo de drift de protocolo.

### La regla que evita que esto se pudra

> En `core` solo entra lo que **no tiene semántica de agente**.

El modo de fallo de todos los cores compartidos es el mismo: se convierten en un god-crate y
acabas obligando a los cinco agentes a subir de versión a la vez. Cuando alguien quiera meter
ahí algo específico de métricas o de deploy, la respuesta es no. Semver estricto y cada agente
pinea su versión.

**Efecto secundario bueno:** un crate publicado *es* un artefacto OSS. "El framework sobre el
que están construidos nuestros agentes" vende mejor que cualquiera de los agentes por separado.

### Nombre

- [ ] Decidir: `sb-agent-core` (claro, ligado a la marca) vs. un nombre neutro que invite a
      adopción externa. Publicar en crates.io desde `securyblack/agent-core`.

---

## Lo que no es Rust

- **Instaladores.** Dejar de mantener un script por repo: **un solo** `install.sh`/`install.ps1`
  parametrizado por nombre de agente. Es exactamente lo que ya insinúa el README de Nexus con
  `install.securyblack.dev/nexus-agent`. Un repo, un script, cinco agentes.
- **Release workflow.** GitHub soporta *reusable workflows* (`workflow_call`) entre repos.
  Un `release.yml` central que cada agente invoca con su nombre y sus targets.

Los dos son ganancia pura sin riesgo de tocar Rust. Van primero.

---

## TUI

Tres cosas distintas se confunden bajo la palabra "TUI". Hay que separarlas:

| | Qué es | Veredicto |
|---|---|---|
| **1. TUI local de un agente** | `oxipulse top` en el host: estado en vivo, buffer, config cargada, último error | **Sí**, en `core` |
| **2. Dashboard multi-agente del host** | Todos los agentes SB de esa máquina | **Sí**, es trabajo de **Nexus** (ya tiene `registry`) |
| **3. TUI de flota** | Cliente de terminal contra la nube SB, todos los servidores | **No.** Duplica la app web y compite consigo misma por mantenimiento |

### El giro: no es un proyecto de UI, es un contrato de estado

> Lo que falta no son widgets. Falta que cada agente **exponga su estado**.

En `sb-agent-core`: un **socket local de status** (Unix socket / named pipe en Windows) que
sirve un JSON pequeño. A partir de ahí sale todo gratis:

- `<agente> status` → una foto, scriptable, para soporte y para `grep`.
- `<agente> top` → la TUI, renderizando ese mismo JSON.
- **Nexus lee los sockets de todos los agentes** → dashboard del host (caso 2) sin código nuevo por agente.
- **El `registry` de Nexus deja de adivinar.** Su plan actual es detectar agentes "chequeando
  proceso, puerto 4317, socket Unix" — heurística frágil. Con el socket es un contrato real.
- **CromoForge se lleva el mejor caso de uso:** `cromo-forge top` mostrando
  build → push → pull → healthcheck → *live*. Eso *es* la experiencia Railway en terminal,
  y es la mejor demo disponible.

**Stack:** `ratatui` + `crossterm`. CupraFlow ya tiene un `src/cli.rs`, así que precedente hay.

**Aviso:** la TUI es acabado, no función — timeboxearla, y que no se cuele delante de
CromoForge v1. Pero el **status socket sí debe aterrizar pronto**: es arquitectura, no
decoración, y cuanto más tarde, más agentes hay que retrofitear.

---

## Hallazgo: el proto tiene deploy y contradice a CromoForge

`nexus-agent/proto/tunnel/v1/tunnel.proto` **ya contiene** `DeployCommand`, `GitPullAction`,
`DockerBuildAction`, `DockerComposeAction`, `DeployLog` y `DeployStatus`.

Ese diseño es **incompatible** con lo que cerramos para CromoForge:

| El proto asume | CromoForge decidió |
|---|---|
| Comandos imperativos (`git_pull`, `docker_build`, `docker_compose`) | Reconciliación de estado deseado |
| Build en el host del cliente | Artefacto = imagen OCI; el host solo hace `pull` |
| Docker Compose | API de Docker directa (por el rollback) |
| Sin concepto de secretos | Secretos sellados X25519, fuera del builder |

**No reutilizar esos mensajes tal cual.** Hay que rediseñar la parte de deploy del Conduit
Protocol al mover el `.proto` a `sb-conduit`. `DeployLog` y `DeployStatus` sí se aprovechan
casi enteros; `DeployCommand` y sus tres acciones hay que reemplazarlos por un mensaje de
estado deseado.

---

## Orden de trabajo

1. [x] **Instalador unificado + release workflow reutilizable.** Hecho 2026-08-21:
       `release.yml` reutilizable + `install-lib.sh`/`install-lib.ps1` en este repo,
       consumidos por OxiPulse, FerroSentry, Nexus Agent y (parcialmente) CupraFlow.
       Pendiente de verificar en el primer tag real (nada se ha pusheado todavía).
       Abierto: CupraFlow no tiene build Linux — su instalación por `nexus-agent`
       en Linux queda deshabilitada con aviso hasta que exista ese target.
2. [x] **`sb-agent-core` v0.1** con lo trivialmente compartible: config, logging, servicio,
       updater, status socket. Hecho 2026-08-23 — `src/config.rs`, `logging.rs`, `service.rs`
       (consola + Windows Service, parametrizado por nombre de servicio y run loop),
       `updater.rs` (parametrizado por repo/bin_name), `buffer.rs` (`OfflineBuffer<T>` +
       `Backoff`, genéricos sobre el tipo de snapshot), `net.rs` (reachability TCP),
       `status.rs` (socket Unix / named pipe Windows, JSON con campos fijos + `details`
       libre). Compila y los 9 tests unitarios pasan en Windows. Sin publicar en crates.io
       todavía — el primer consumidor real (punto 3) es lo que validará la API.
3. [ ] **CromoForge como primer consumidor.** Es greenfield: nace sobre `core` desde el día uno
       y **valida la API antes** de tocar los otros cuatro. Si `core` está mal diseñado, te
       enteras con un agente, no con cinco.
4. [ ] **`sb-conduit`** cuando CromoForge necesite el túnel — ahí habrá tres consumidores y el
       diseño correcto será obvio. Incluye el rediseño de la parte de deploy del proto.
5. [~] **Retrofit uno a uno**, empezando por FerroSentry (el más parecido a OxiPulse).
       Nunca big-bang. **FerroSentry hecho 2026-08-23** (4 pasos: config, updater,
       logging, servicio+status socket) — encontró y arregló dos bugs reales de paso:
       `log_level` se cargaba pero nunca se aplicaba (init corría antes que la config),
       y `service::windows::run_service` exigía `Send` en el future sin necesitarlo
       (rompía código legítimo con un `MutexGuard` retenido a través de un `.await`).
       **OxiPulse hecho 2026-08-23** (mismos 4 pasos, + `buffer.rs`/`is_reachable`/
       `parse_host_port` movidos a `sb_agent_core::buffer`/`::net` ya que OxiPulse era
       el origen literal de ese código). De paso arregló un bug preexistente y sin
       relación — `opentelemetry`/`opentelemetry-otlp` en 0.27 vs `opentelemetry_sdk`
       en 0.32, ya roto en el `Cargo.lock` comitteado antes de tocar nada del retrofit.
       **Nexus Agent hecho 2026-08-23** (mismos 4 pasos). Conservó a propósito su
       `STARTUP_DELAY` de 300s (vs 60s de los demás) vía `with_startup_delay()` — es
       el drift original que motivó todo este crate, y homogeneizarlo sin que nadie
       lo pida sería una decisión de producto, no un detalle de refactor. También
       cayó el feature opcional `windows-service` de Cargo, redundante ahora que
       `sb-agent-core` ya trae esa dependencia siempre en Windows.
       **CupraFlow hecho 2026-08-23** — retrofit parcial, a propósito: solo config
       (carga + sync de versión) y status socket. No tenía updater que retrofitear
       (nunca implementó auto-update pese a tener la sección `[update]` en config), y
       su registro de servicio vía `ServiceManager`/subcomandos del binario es
       genuinamente distinto al `New-Service` de los otros tres — no se tocó. De paso
       arregló el mismo bug que FerroSentry: `logging.level` se cargaba pero nunca se
       aplicaba. **Los 4 agentes existentes retrofiteados — este punto queda cerrado.**
6. [x] **TUI** sobre el socket. Hecho 2026-08-23:
       - `status_client.rs`: lee un snapshot del socket/pipe, síncrono, con timeout opcional.
       - `tui.rs`: `run_top(agent_name)` — vista ratatui genérica (estado/versión/uptime +
         `details` como JSON), refresco ~1s, `q`/Esc para salir. No sabe nada de métricas,
         hallazgos ni fases de deploy — cada agente se lleva esto gratis.
       - `cli.rs`: `dispatch_common_args` — unifica `--version`/`status`/`top` en un único
         punto en vez de copiarlo en cada `main.rs` (estaba a punto de repetirse una cuarta
         vez cuando lo generalicé).
       - Cableado en los 4 agentes: OxiPulse, FerroSentry y Nexus Agent vía
         `dispatch_common_args`; CupraFlow por su cuenta (tiene su propio `clap`) con un
         subcomando `top` nuevo y `status` ampliado con la foto del socket.
       - **Nivel 2 cerrado también**: `nexus-agent/src/registry/mod.rs` ahora prueba el
         status socket antes de la heurística de proceso/PATH/`--version` por subproceso —
         si el socket responde es autoritativo (proceso vivo, versión real, cero subprocesos).
         La heurística vieja solo sobrevive como fallback para distinguir "parado" de
         "nunca instalado", algo que el socket solo no puede decir.
       - Verificado con un test de extremo a extremo real (servidor + cliente, socket real)
         en `tests/status_roundtrip.rs`, aparte de probar en caliente contra un OxiPulse
         real corriendo en la máquina de desarrollo.
       - **Nivel 3 (TUI de flota) sigue descartado**, como decía el diseño original.
7. [~] **Intake de comandos** — contraparte del status socket, en la otra dirección. Ver
       `D:\infra\docs\design-command-intake.md` para el porqué (cada agente ejecuta lo suyo;
       Nexus enruta, no interpreta). Módulo `command_intake.rs` hecho 2026-08-24: socket/pipe
       separado del de status, `CommandRegistry` con handlers registrados por `command_type`
       (payload libre, el core no lo interpreta), timeout por comando, idempotencia por
       `command_id` (últimos 256 recordados), y `CommandProgress` para operaciones largas
       (streaming línea a línea antes del `CommandResponse` final). `command_intake_client.rs`
       — cliente síncrono, mismo estilo que `status_client.rs`, para que Nexus lo llame desde
       `spawn_blocking` al reenviar un comando del túnel al intake local del agente destino.
       Verificado con test de extremo a extremo real (socket real, progreso incluido) en
       `tests/command_intake_roundtrip.rs`, además de los 4 tests unitarios de `dispatch()`.
       **Fix 2026-08-24:** `dispatch()` no sellaba `command_id` en los `CommandProgress` que
       reenvía — el handler no lo conoce y no debería tener que pasarlo. Ahora `dispatch()`
       lo sella él mismo sobre cada progreso antes de reenviarlo, en vez de confiar en que
       cada handler (o cada consumidor que reenvíe progreso, como nexus) lo haga bien.
       Primer consumidor real: handler `os_upgrade` en FerroSentry, hecho 2026-08-24.
       **Autenticación hecha 2026-08-24** — módulo nuevo `intake_auth.rs`: token compartido de
       32 bytes aleatorios en `/etc/sb-agent/intake.token` (Unix, fichero `0600`) o
       `%ProgramData%\sb-agent\intake.token` (Windows), generado por el primer agente que
       arranca (creación atómica con `create_new`, para que dos agentes arrancando a la vez no
       se pisen el token — bug real encontrado en los tests de integración, que corrían en
       paralelo contra el mismo fichero). `CommandEnvelope` lleva ahora `auth_token: String`;
       `command_intake_client::send_command` lo rellena solo (el llamante no necesita saber que
       existe), y el servidor lo compara en tiempo constante antes de tocar `dispatch()` — un
       envelope sin token válido nunca llega al registro de handlers. Sin esto el servidor del
       intake ni siquiera arranca (falla `ensure_token()` → no se hace `bind`/`listen`).
       Verificado con un test nuevo de rechazo (`command_intake_rejects_wrong_auth_token`) que
       escribe el envelope a mano saltándose el cliente (que siempre estampa el token correcto).
       **Deliberadamente fuera de esta pasada:** el grupo Unix compartido que el documento de
       diseño mencionaba como defensa en profundidad extra — requiere decisiones de packaging
       (crear el grupo `sb-agents`, añadir los usuarios de servicio correctos) que no tocan este
       crate; el token ya cierra el hueco de autenticación por sí solo en ambos SO.

---

## Abierto

- [ ] Nombre y ubicación del crate (`sb-agent-core` vs. neutro).
- [ ] ¿`core` publica en crates.io o se consume por git tag? Crates.io es más limpio para OSS
      y obliga a disciplina de release; git tag es más rápido al principio.
- [x] Esquema del JSON de status — cerrado en `status.rs`: `{agent, version, state, since_unix,
      details}`, campos fijos + blob libre.
- [x] El `registry` de Nexus ya consume el status socket como fuente primaria — hecho junto con
      la TUI (punto 6). La heurística vieja queda solo como fallback.
