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
    qml/       La UI. Vive dentro del crate, no en la raíz: cxx-qt escribe las
               rutas del `qmldir` tal cual se le dan, y un `../..` sale del
               módulo y rompe la carga en tiempo de ejecución.
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
/usr/lib/qt6/bin/qmllint -I target/cxxqt/qml_modules crates/kara-ui/qml/*.qml
```

Sin el `-I` falla con "Failed to import com.kara.ui", que es un falso positivo.

## Concurrencia: el lock del árbol

Más de un proceso puede escribir aquí: los workflows `kara-tdd`, que lanzan
agentes que editan ficheros y hacen commits, y la sesión interactiva desde la que
los lanzas. Dos a la vez producen commits entrelazados y builds corruptos —
ocurrió el 2026-09-02. **Antes de escribir en el árbol, toma el lock:**

```bash
./scripts/kara-lock acquire <dueño>   # 1 = ocupado, no insistas
./scripts/kara-lock refresh <dueño>   # entre pasos largos, o caduca
./scripts/kara-lock release <dueño>   # siempre, incluso al abortar
```

Caduca **solo por latido** (60 min por defecto), nunca por liveness del PID: cada
llamada bash abre una shell que muere al terminar, así que el PID registrado
siempre está muerto en la llamada siguiente y usarlo dejaba que cualquiera
robase el lock al instante. La contrapartida es que un proceso que muera sin
liberar bloquea el árbol hasta que caduque el TTL.

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

**Language: English, from here on.**
- All new source code — identifiers, comments and doc comments — and all new
  documentation (this file, READMEs, commit messages) are written in English.
- **Do not translate or rewrite what already exists.** A large part of the tree
  was written in Spanish; it stays as it is. Touch a Spanish comment only when
  the code under it changes anyway, and even then don't turn a working file into
  a translation diff.
- `ground/spec/` is frozen reference material and stays in Spanish.
- User-facing strings in the UI are a separate matter from this rule: they are
  Spanish today because there is no i18n yet.

**Trabajo.**
- Construye en el orden de `08-prioridades.md`. No empieces un `should` con `must`
  pendientes.
- El listado de un directorio nunca bloquea la UI, ni con 100 k entradas ni con un
  volumen de red colgado.

## Estado

**Backend: los cuatro pilares en pie.** `kara-core` (ordenación con collation,
filtros, historial, breadcrumb, autocompletado, typeahead), `kara-fs` (listado,
papelera FreeDesktop, copiar/mover/renombrar), `kara-ops` (cola con progreso,
velocidad y ETA, pila de deshacer, resolución de conflictos) y `kara-index`
(travesía paralela, búsqueda, vigilancia inotify).

**UI: el chrome montado sobre datos reales.** Ventana sin marco con barra de
título propia, barra de comandos con atrás/adelante/subir/refrescar, migas de
pan navegables con desbordamiento, edición de ruta con Ctrl+L, filtro por nombre
y barra de estado. Panel de navegación con Acceso rápido (XDG) y Este equipo
(raíz y volúmenes montados), árbol con carga diferida que sigue a la carpeta que
se enseña. El listado sale de `kara-fs`, no de datos falsos.

**Iconos: los del escritorio, no un juego propio.** `kara-fs::mime` resuelve el
tipo con `/usr/share/mime/globs2` —la misma base que Dolphin y Nautilus— y
`kara-fs::icons` implementa la búsqueda de temas de FreeDesktop (herencia,
tallas, `Fixed`/`Scalable`/`Threshold`). Lo que el usuario tenga puesto es lo que
sale, incluidas las carpetas especiales del panel.

La columna «Tipo» dice lo que dice el sistema («Documento JSON»), traducido por
la propia base de FreeDesktop. El idioma está fijado a español porque el resto
de la ventana lo está: cuando la UI tenga traducciones, esa constante pasa a ser
la cadena de idiomas del entorno.

**Miniaturas: la caché compartida del escritorio.** `kara-fs::thumbnails`
implementa el estándar de FreeDesktop —`~/.cache/thumbnails`, nombre = MD5 del
`file://` URI, `Thumb::URI` y `Thumb::MTime` dentro del PNG—, así que lo que
Dolphin ya generó se ve al instante y lo que Kara genera lo aprovechan los
demás. Se leen los tres tipos de chunk de texto (`tEXt`, `zTXt`, `iTXt`): hay
generadores reales que comprimen el URI. La búsqueda y la generación van en un
hilo aparte, en dos pasadas —primero la caché, luego generar—, y un número de
listado invalida el trabajo en cuanto el usuario cambia de carpeta.

`kara-ui` acepta la carpeta a enseñar como argumento: `cargo run -p kara-ui --
~/Descargas`.

**Lo que falta de la vista:** los cuatro modos de vista con zoom,
ordenar pulsando en la cabecera, selección múltiple, anclar carpetas al panel
(necesita dónde guardar ajustes), y enchufar a la UI lo que el backend ya ofrece
y nadie consume todavía (vigilancia, cola, deshacer).

### Cabos sueltos conocidos

- **Enlazado con `ld.bfd`**: el build avisa de que no hay `mold`, `lld` ni `gold`.
  Funciona, pero un `sudo apt install mold` acorta bastante el ciclo de compilación.
- **Todo el I/O de listado es síncrono**, y desplegar el panel lo multiplica por
  la profundidad de la ruta. Una carpeta enorme o un volumen de red colgado
  bloquean la ventana. La spec lo prohíbe explícitamente: el listado asíncrono
  es trabajo pendiente, no un detalle.
- **El binario `qml` de Qt no está**: `qt6-declarative-dev-tools` trae `qmllint`,
  `qmlformat` y `qmlls`, pero no el runtime suelto. Irrelevante para Kara (cxx-qt
  embebe el motor); solo afecta a `ground/run.sh`, que cae a PyQt6.
