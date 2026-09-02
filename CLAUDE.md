# Kara — explorador de ficheros

Alternativa a Dolphin para Linux: la **estética y las comodidades del Explorador de
Windows 11** sobre un stack nativo, respetando las convenciones POSIX y FreeDesktop
(papelera, MIME, miniaturas). Objetivo declarado: ambicioso — cubrir las 142
conveniencias especificadas, no un visor de carpetas.

## Fuente de verdad

`ground/spec/` **es la especificación del producto**. 142 conveniencias, 196 atajos,
fundamentadas en Windows 11, Dolphin, Nemo, Nautilus, Finder, Directory Opus y Total
Commander. No inventes comportamiento: si vas a implementar algo, localiza primero su
conveniencia y respeta sus casos borde.

| Documento | Contenido |
|---|---|
| `ground/spec/00-filosofia.md` | Principios. **Léelo antes de tomar decisiones de UX.** |
| `ground/spec/01-navegacion.md` … `06-contexto-power.md` | Los seis dominios |
| `ground/spec/07-atajos-teclado.md` | Keymap maestro — la asignación de teclas sale de aquí |
| `ground/spec/08-prioridades.md` | **Orden de construcción.** must → should → could |

`ground/` es **material de referencia: no se modifica**. Contiene además un spike QML
(`Main.qml`, `FolderTree.qml`, `Theme.qml`, `util.js`) con datos falsos que valida la
estética Fluent y de donde se migra el QML real.

## Stack

**Rust + Qt Quick/QML**, puente vía `cxx-qt`. La UI es QML declarativo; toda la lógica
vive en Rust.

Por qué, medido en esta máquina (no de folleto):

- Listar 50 000 entradas: Python 150 ms vs nativo ~70–200 ms → **empate**
  (`os.scandir` es C). El rendimiento de listado *no* fue el motivo.
- Travesía recursiva de `/usr` (251 k ficheros): Python 2,29 s vs `find` 0,32 s vs
  paralelo 0,15 s → **7–15×**. Afecta a búsqueda recursiva, tamaño de carpeta y la
  fase "Calculando…" previa a copiar.
- **Motivo principal: seguridad de las operaciones destructivas.** La spec gira sobre
  "reversibilidad por defecto"; ese código quiere un compilador que obligue a tratar
  cada fallo de syscall. Un `except OSError` olvidado borra datos del usuario.

## Entorno (verificado)

- Ubuntu 26.04 · **KDE Plasma sobre Wayland** · 24 cores · 30 GB RAM
- Qt **6.10.2** con 75 módulos QML de runtime ya instalados
- Rust 1.98 · Node 22 · Python 3.14 (GIL activo)
- Dolphin, `kioclient` y 217 paquetes KF presentes → referencia de comportamiento a mano

**Prerrequisito de toolchain.** Sin esto `rustc` ni siquiera enlaza (falta `cc`):

```bash
sudo apt install -y build-essential cmake ninja-build pkg-config qt6-base-dev qt6-declarative-dev qt6-declarative-dev-tools qt6-svg-dev
```

Requiere `sudo`: pídeselo al usuario, no intentes ejecutarlo tú.

## Arquitectura

```
crates/
  kara-core/   Dominio puro: entrada de fichero, ordenación, agrupación, filtros.
               Sin I/O, sin Qt. Es la capa que se testea a fondo.
  kara-fs/     Syscalls: scandir/stat, copiar, mover, enlazar, permisos POSIX,
               papelera FreeDesktop (.trashinfo). Todo devuelve Result.
  kara-index/  Travesía paralela (jwalk/rayon), búsqueda, watcher inotify (notify).
  kara-ops/    Cola de operaciones, progreso con velocidad/ETA, resolución de
               conflictos, pila de deshacer/rehacer.
  kara-ui/     Puente cxx-qt + binario principal. Expone modelos a QML.
qml/           Main.qml, FolderTree.qml, Theme.qml — migrados del spike.
```

Regla de capas: `ui → ops → {fs, index} → core`. Nunca al revés.

## Comandos

```bash
cargo run -p kara-ui        # arrancar
cargo test                  # tests (el grueso vive en kara-core y kara-fs)
cargo clippy --all-targets  # antes de dar nada por terminado
./ground/run.sh             # spike QML de referencia (datos falsos)
```

Para revisar el QML hace falta apuntar a la ruta del módulo que genera cxx-qt, y
`qmllint` no está en el `PATH` (vive en `/usr/lib/qt6/bin`):

```bash
/usr/lib/qt6/bin/qmllint -I target/cxxqt/qml_modules qml/Main.qml
```

Sin el `-I` falla con "Failed to import com.kara.ui", que es un falso positivo.

## Reglas del proyecto

**Operaciones de fichero — no negociable.**
- Eliminar va **siempre** a la papelera FreeDesktop. El borrado permanente
  (Shift+Supr, vaciar papelera) **confirma** y el foco arranca en el botón seguro.
- Mover, copiar, renombrar y crear entran en la pila de deshacer.
- Prohibido `unwrap()` / `expect()` en cualquier ruta que toque ficheros del usuario.
  Cada syscall devuelve `Result` y cada error se propaga o se reporta.
- Un fallo en un lote **no aborta el lote**: reintentar / omitir / cancelar, y resumen
  final de errores.
- Nada de operaciones silenciosas: fase "Calculando…", progreso, y conflictos
  resueltos explícitamente (Reemplazar / Omitir / Conservar ambos / Aplicar a todos).

**UI.**
- QML **no** contiene lógica de negocio: solo presentación y binding a modelos Rust.
- Los atajos salen de `07-atajos-teclado.md`. Convención Windows como principal, y
  donde hay conflicto de tradiciones (Retroceso = Atrás vs Subir) se deja configurable.
- El estado se recuerda: vista, orden, columnas y zoom persisten **por carpeta**;
  Atrás restaura scroll y selección.

**Trabajo.**
- Construye en el orden de `08-prioridades.md`. No empieces un `should` con `must`
  pendientes.
- Documentación y spec en español; código, identificadores y comentarios en inglés.
- El listado de un directorio nunca bloquea la UI, ni con 100 k entradas ni con un
  volumen de red colgado.

## Estado

Andamiaje montado y verificado: repositorio git inicializado (rama `main`), workspace
Cargo con los cinco crates, puente `cxx-qt` 0.10 funcionando y binario que arranca y
carga QML. Sin funcionalidad todavía: `qml/Main.qml` es una ventana placeholder y los
crates de backend están vacíos.

Siguiente paso: migrar la UI del spike (`ground/Main.qml`, `FolderTree.qml`,
`Theme.qml`, `util.js`) a `qml/`, quitando el fondo flotante y los `anchors.margins`
que lo hacen parecer una captura, y sustituir los datos falsos por un modelo real
alimentado desde `kara-fs`.

### Cabos sueltos conocidos

- **Identidad de git sin configurar** (`user.name` / `user.email`): no se puede hacer
  commit hasta que el usuario la fije.
- **Enlazado con `ld.bfd`**: el build avisa de que no hay `mold`, `lld` ni `gold`.
  Funciona, pero un `sudo apt install mold` acorta bastante el ciclo de compilación.
- **El binario `qml` de Qt no está**: `qt6-declarative-dev-tools` trae `qmllint`,
  `qmlformat` y `qmlls`, pero no el runtime suelto. Irrelevante para Kara (cxx-qt
  embebe el motor); solo afecta a `ground/run.sh`, que cae a PyQt6.
