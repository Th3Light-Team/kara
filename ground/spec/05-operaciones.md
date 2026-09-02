# 5. Operaciones de fichero y feedback

> Especificación de *conveniencias* (qué y cómo se comporta), agnóstica de implementación.

Especificación exhaustiva de conveniencias de cara al usuario para operaciones de fichero (cortar/copiar/pegar, arrastrar y soltar, copiar, mover, renombrar, eliminar, restaurar) y su feedback: diálogos de progreso con velocidad/tiempo restante/pausar/cancelar, fase de preparación, resolución de conflictos (reemplazar/omitir/mantener ambos), combinación de carpetas, manejo de errores (reintentar/omitir/cancelar), papelera y restauración, eliminación permanente, deshacer/rehacer, propiedades y permisos, cálculo de tamaño de carpeta y notificaciones de fin. Cada conveniencia está fundamentada en al menos un explorador real (Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files/Nautilus, macOS Finder, Directory Opus, Total Commander) y orientada a un explorador moderno con estética y comodidades tipo Windows 11 en Linux (modelo de permisos POSIX). Sin detalles de implementación: solo comportamiento esperado y casos borde, con atajos verificados y variantes por explorador anotadas en 'context'/'notes'.

---

## Diálogo de progreso de copia/movimiento

**Categoría:** Progreso y transferencia  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files, macOS Finder, Directory Opus, Total Commander

Ventana o barra que aparece durante copias, movimientos o eliminaciones largas, mostrando progreso global, elemento actual y control para cancelar. Es el feedback central de cualquier operación no instantánea.

**Comportamiento esperado:** Aparece solo cuando la operación supera un umbral corto (~0,5-1 s) para no parpadear en tareas triviales. Antes de transferir, muestra una fase de preparación/enumeración ('Calculando…' / 'Descubriendo elementos') mientras recorre el árbol de origen, sin barra determinista todavía. Luego muestra: barra de progreso porcentual, nombre del elemento en curso, contador 'X de Y elementos' y volumen transferido (p. ej. 340 MB de 1,2 GB). Debe incluir siempre un botón de Cancelar visible; Esc equivale a cancelar el diálogo enfocado. Al cancelar, la operación se detiene de forma segura (sin dejar el archivo en curso a medio escribir) e informa qué se completó y qué no. Casos borde: operaciones de 0 bytes o con muchísimos archivos pequeños deben reflejar progreso por conteo de elementos, no solo por bytes; si el destino se queda sin espacio, el diálogo debe pausar y ofrecer reintentar/omitir/cancelar en vez de fallar en silencio; una copia entre dos rutas del mismo sistema de ficheros es un movimiento instantáneo (renombrado) y no debe recorrer bytes.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Esc` | Cerrar/cancelar el diálogo de progreso enfocado |

> ℹ️ En Windows 11 la barra es compacta con expansor 'Más detalles' y muestra la fase 'Calculando…' antes de copiar. En GNOME Files/Nautilus el progreso puede minimizarse a un botón/indicador en la barra de herramientas mientras se sigue navegando. Total Commander y Directory Opus muestran diálogos más ricos con logs por archivo.

## Pausar y reanudar operación

**Categoría:** Progreso y transferencia  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Directory Opus, Total Commander

Botón para poner en pausa una transferencia en curso y reanudarla después, útil para liberar disco/red temporalmente sin perder el progreso.

**Comportamiento esperado:** El botón de pausa detiene la transferencia dejándola en un estado seguro (sin corromper el archivo en curso) y cambia a un botón de reanudar; el tiempo restante y la velocidad se marcan como 'En pausa'. Debe poder pausarse cada operación por separado cuando hay varias concurrentes. Casos borde: al reanudar tras mucho tiempo, revalidar que origen y destino siguen existiendo (unidad extraíble desconectada, archivo movido) y avisar en vez de fallar; la pausa no debe bloquear el resto del explorador; una operación en pausa mucho tiempo no debe mantener bloqueado (lock) el archivo destino de forma que impida a otras apps usarlo sin aviso.

> ℹ️ Windows 11 y Dolphin exponen pausa/reanudar explícita en el diálogo. GNOME Files/Nautilus y macOS Finder NO ofrecen pausa real, solo cancelar. No hay atajo de teclado estándar; se opera con el botón del diálogo.

## Velocidad, tiempo restante y gráfico de transferencia

**Categoría:** Progreso y transferencia  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Directory Opus, Total Commander

Indicadores de velocidad instantánea (MB/s), tiempo estimado restante y, opcionalmente, un pequeño gráfico de velocidad en el tiempo dentro del diálogo de progreso.

**Comportamiento esperado:** Muestra velocidad actual y ETA recalculados de forma suavizada para no dar cifras erráticas; el ETA debe redondearse de forma legible ('Aprox. 2 minutos restantes', no '00:01:57'). Cuando la velocidad cae a cero (esperando disco/red) debe reflejarlo en vez de congelar una cifra antigua. El gráfico de velocidad se muestra al expandir detalles. Casos borde: en copias de muchos archivos minúsculos la velocidad en MB/s baja aunque el avance sea rápido; conviene complementar con elementos/segundo. Evitar ETAs sensacionalistas tipo '99 horas' al arrancar: mostrar 'Calculando…' hasta que la estimación se estabilice.

> ℹ️ El GRÁFICO de velocidad es específico de Windows 8/10/11 al pulsar 'Más detalles'. Dolphin muestra velocidad y tiempo restante solo en texto. GNOME Files/Nautilus y Finder muestran ETA/velocidad de forma más escueta y sin gráfico. La velocidad+ETA (sin gráfico) sí es común a los cuatro.

## Detalles expandibles del progreso ('Más/Menos detalles')

**Categoría:** Progreso y transferencia  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer

Expansor que amplía el diálogo compacto para revelar ruta de origen y destino, nombre exacto del archivo, elementos restantes y el gráfico de velocidad.

**Comportamiento esperado:** Por defecto se muestra la vista compacta; el usuario puede expandir para ver 'De: <ruta>  A: <ruta>', archivo actual e ítems pendientes. El estado expandido/colapsado debe recordarse entre operaciones dentro de la sesión. Casos borde: rutas muy largas deben truncarse por el medio con tooltip del valor completo; el diálogo no debe crecer fuera de pantalla al expandirse.

> ℹ️ GROUNDING: el patrón concreto de colapsar/expandir 'Más detalles' / 'Menos detalles' es de Windows 8/10/11. Directory Opus y Total Commander muestran esos mismos datos de forma PERMANENTE (sin un expansor tipo Windows), por eso se retiran de seenIn aquí; su detalle rico se refleja en 'copy-move-progress-dialog'. Comodidad muy asociada a la estética Windows 11 que se busca replicar.

## Operaciones concurrentes agrupadas y cola de transferencia

**Categoría:** Progreso y transferencia  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, GNOME Files, KDE Dolphin, Directory Opus, Total Commander

Cuando hay varias copias/movimientos a la vez, se muestran juntas en una sola ventana (o panel apilado) con una barra por operación, en lugar de abrir muchas ventanas dispersas; idealmente con posibilidad de encolar.

**Comportamiento esperado:** Iniciar una segunda operación mientras otra corre debe añadir una fila/tarjeta al mismo contenedor, con su propio progreso, pausa y cancelación. Debe existir un progreso combinado o al menos un resumen ('2 operaciones en curso'). Casos borde: cancelar una no debe afectar a las demás; cerrar el contenedor con operaciones activas debe advertir; idealmente permitir ENCOLAR (ejecutar en serie) en vez de saturar el mismo disco con transferencias simultáneas que se ralentizan entre sí.

> ℹ️ Windows 11 consolida varias copias en una ventana con varias barras. GNOME Files/Nautilus apila operaciones en un indicador/menú. Total Commander y Directory Opus ofrecen COLAS de transferencia explícitas (encolar en lugar de paralelizar), muy valoradas por usuarios avanzados.

## Progreso en el icono de la barra de tareas

**Categoría:** Progreso y transferencia  
**Prioridad:** ⚪ Opcional  
**Visto en:** Windows 11 File Explorer, KDE Dolphin

El icono del explorador en la barra de tareas se rellena o colorea según el avance de la operación, dando feedback sin tener el diálogo en primer plano.

**Comportamiento esperado:** Mientras hay una operación activa, el icono muestra un relleno proporcional al progreso global; al terminar vuelve al estado normal. Si una operación queda a la espera de una decisión (conflicto, error), el indicador debe cambiar de color/estado para llamar la atención (p. ej. ámbar/pausa). Casos borde: con varias operaciones, reflejar el progreso agregado; el indicador debe limpiarse si la ventana se cierra o la operación se cancela.

> ℹ️ Nativo en Windows (progreso verde sobre el icono; ámbar cuando requiere atención). En KDE/Plasma, la entrada de Dolphin en la barra de tareas refleja el progreso del trabajo vía el sistema de 'Jobs' de Plasma. GNOME no lo hace de forma estándar. Encaja con la estética Windows 11 objetivo.

## Cortar, copiar y pegar (portapapeles)

**Categoría:** Operaciones básicas de archivo  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files, macOS Finder, Directory Opus, Total Commander

Copiar (Ctrl+C) o cortar (Ctrl+X) la selección al portapapeles y pegarla (Ctrl+V) en otra carpeta, con realimentación visual de los elementos cortados.

**Comportamiento esperado:** Ctrl+C marca la selección para copiar y Ctrl+X para mover; al cortar, los iconos se muestran atenuados/fantasma hasta que se pega o se cancela (Esc). Ctrl+V pega en la carpeta activa lanzando, si procede, el diálogo de progreso y el de conflictos. Pegar en la MISMA carpeta de origen genera una copia con sufijo predecible ('nombre - copia' / 'nombre (2)') en vez de fallar o sobrescribir. Cortar y no pegar (o pegar en un destino inválido) NO debe borrar el origen; si entre el corte y el pegado el origen cambió o desapareció, avisar. Casos borde: en macOS Finder NO existe 'cortar' de archivos con Cmd+X: se copia con Cmd+C y se MUEVE al pegar con Cmd+Option+V ('Mover ítem aquí'); pegar un elemento previamente cortado vacía el portapapeles (no se puede pegar dos veces un 'cortar').

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+C` | Copiar la selección. macOS Finder: Cmd+C |
| `Ctrl+X` | Cortar (mueve al pegar), iconos atenuados. macOS Finder no corta archivos con Cmd+X |
| `Ctrl+V` | Pegar. macOS Finder: Cmd+V copia; Cmd+Option+V mueve ('Mover ítem aquí') |

> ℹ️ El atenuado del icono al cortar es propio de Windows y de los exploradores Linux. Finder rompe el modelo cortar/pegar: usa copiar + 'Mover ítem aquí' (Cmd+Option+V). Los gestores de doble panel Total Commander y Directory Opus además copian/mueven con F5 (copiar) y F6 (mover) entre paneles, herencia de Norton Commander.

## Arrastrar y soltar con modificadores (copiar/mover/enlace)

**Categoría:** Operaciones básicas de archivo  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files, macOS Finder, Directory Opus, Total Commander

Arrastrar la selección a otra carpeta, pestaña o panel para moverla o copiarla, con teclas modificadoras que fuerzan la acción y un distintivo en el cursor que anticipa qué ocurrirá.

**Comportamiento esperado:** Por defecto, arrastrar dentro del MISMO volumen mueve y entre volúmenes DISTINTOS copia; el cursor debe mostrar un distintivo claro (flecha = mover, '+' = copiar, badge de enlace = crear acceso directo). Modificadores en Windows/Linux: Ctrl fuerza copiar, Shift fuerza mover, Ctrl+Shift (o Alt en Windows) crea enlace/acceso directo. Soltar debe resaltar la carpeta destino bajo el cursor. Casos borde: soltar sobre el propio directorio de origen es no-op; arrastrar con el BOTÓN DERECHO abre un menú al soltar con 'Copiar aquí / Mover aquí / Crear enlace aquí / Cancelar'; soltar sobre una carpeta sin permiso de escritura debe avisar en vez de fallar en silencio; una operación de arrastre larga usa el mismo diálogo de progreso y puede cancelarse con Esc.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl (durante el arrastre)` | Forzar COPIAR. macOS Finder: Option fuerza copiar |
| `Shift (durante el arrastre)` | Forzar MOVER. macOS Finder: Cmd fuerza mover |
| `Ctrl+Shift o Alt (durante el arrastre)` | Crear enlace/acceso directo. macOS Finder: Cmd+Option crea un alias |

> ℹ️ GROUNDING: macOS INVIERTE los modificadores respecto a Windows (Cmd mueve, Option copia, Cmd+Option = alias). Dolphin muestra por defecto un menú al soltar (Copiar/Mover/Enlazar/Cancelar). El arrastre con botón derecho que abre menú es patrón clásico de Windows.

## Renombrar en línea y renombrado por lotes

**Categoría:** Operaciones básicas de archivo  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files, macOS Finder, Directory Opus, Total Commander

Renombrar un elemento in situ con F2 y renombrar muchos a la vez con numeración/buscar-reemplazar y vista previa en vivo.

**Comportamiento esperado:** F2 activa la edición del nombre con el cuerpo (SIN la extensión) preseleccionado para no borrarla por error; Esc cancela, Enter confirma. Con varios elementos seleccionados, ofrecer renombrado por lotes: patrón base con contador ('foto (1)', 'foto (2)'), buscar/reemplazar y añadir prefijo/sufijo, mostrando una VISTA PREVIA en vivo del resultado antes de aplicar. Casos borde: nombres duplicados resultantes deben resolverse con sufijo o avisar antes de aplicar; caracteres inválidos para el sistema de ficheros deben rechazarse o sustituirse con aviso; conservar extensiones compuestas ('.tar.gz'); el renombrado debe ser deshacible con Ctrl+Z recuperando el nombre exacto anterior; en renombrado en línea, Tab/Shift+Tab debe saltar al siguiente/anterior elemento manteniendo la edición.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `F2` | Renombrar en línea el elemento seleccionado (Windows/Dolphin/Nautilus/Nemo). macOS Finder: Enter/Return |

> ℹ️ GROUNDING: en macOS Finder se renombra con Return (no F2); Return NO abre la carpeta (abrir es Cmd+Down). Windows 11 renombra en lote como 'base (1)', 'base (2)' y Tab pasa al siguiente. Nautilus tiene diálogo 'Renombrar…' por lotes con plantilla/numeración y previsualización. Finder 'Renombrar N ítems' (Reemplazar texto / Añadir texto / Formato con contador) con previsualización. Directory Opus incluye renombrado avanzado con expresiones regulares.

## Resolución de conflictos (reemplazar / omitir / mantener ambos)

**Categoría:** Resolución de conflictos  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files, macOS Finder, Directory Opus, Total Commander

Diálogo que aparece cuando el destino ya contiene un archivo con el mismo nombre, ofreciendo reemplazar, omitir o conservar ambos.

**Comportamiento esperado:** Presenta las tres acciones básicas con lenguaje claro: 'Reemplazar el archivo en el destino', 'Omitir este archivo' y 'Conservar ambos archivos', distinguiendo con nitidez cuál es el origen y cuál el destino. La opción por defecto (foco inicial) debe ser la MENOS destructiva (no reemplazar automáticamente). Casos borde: conflicto entre archivo y carpeta con el mismo nombre debe tratarse aparte (no sobrescribir una carpeta con un archivo sin aviso explícito); mover a la misma carpeta (origen == destino) no es conflicto sino no-op o copia con sufijo; conflictos de solo lectura o sin permiso deben ofrecer omitir en lugar de fallar toda la operación; conflicto carpeta-contra-carpeta debe ofrecer 'Combinar' (ver 'merge-folders').

> ℹ️ Terminología por explorador: Windows 'Reemplazar / Omitir / Comparar información'; macOS Finder 'Reemplazar / Detener / Conservar ambos'; Dolphin ofrece además renombrar en el propio diálogo; Nautilus/Nemo 'Reemplazar / Omitir' y 'Combinar' para carpetas.

## Aplicar la misma acción a todos los conflictos

**Categoría:** Resolución de conflictos  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files, macOS Finder

Casilla o botón para aplicar la decisión elegida (reemplazar/omitir/mantener ambos) a todos los conflictos restantes de la misma operación, evitando responder archivo por archivo.

**Comportamiento esperado:** Al marcar 'Hacer esto para todos los conflictos' (o equivalente), la elección se aplica a lo que reste sin volver a preguntar. Debe permitirse decidir distinto por archivo si NO se marca la casilla. Idealmente ofrecer decisiones separadas por tipo de conflicto (p. ej. 'omitir los idénticos, preguntar el resto'). Casos borde: mostrar cuántos conflictos pendientes hay para que el usuario dimensione la decisión; permitir cambiar de opinión mientras aún queden conflictos por aplicar; el resultado final debe reflejar cuántos se reemplazaron/omitieron/duplicaron.

> ℹ️ Windows 11 lista los conflictos con casillas por archivo para elegir versión (izquierda=origen / derecha=destino) además de una acción global. Nautilus/Nemo y Finder usan una casilla 'Aplicar a todos'. Imprescindible para operaciones masivas.

## Mantener ambos (renombrado automático) y renombrar en conflicto

**Categoría:** Resolución de conflictos  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, macOS Finder, KDE Dolphin, GNOME Files, Cinnamon Nemo

Opción que conserva el archivo existente y el nuevo, renombrando automáticamente el entrante (p. ej. 'informe (2).pdf'), con posibilidad de teclear un nombre manual.

**Comportamiento esperado:** Al elegir 'Conservar ambos', genera un nombre único predecible añadiendo un sufijo numérico ANTES de la extensión. El explorador tipo Windows debe respetar el patrón 'nombre (2).ext', 'nombre (3).ext'. Debe permitir además editar el nombre propuesto manualmente en el diálogo (como hace Dolphin). Casos borde: preservar correctamente extensiones compuestas ('.tar.gz'); si el nombre generado también existe, incrementar hasta encontrar uno libre; nombres que excedan el límite del sistema deben truncar el cuerpo conservando el sufijo y la extensión.

> ℹ️ GROUNDING: en un CONFLICTO, Windows añade ' (2)'; el sufijo ' - copia' / '- Copy' es el que Windows usa al COPIAR-PEGAR en la misma carpeta, no en el diálogo de conflicto (matiz distinto). Finder usa 'Conservar ambos' añadiendo un número. Dolphin ofrece un campo de renombrado con sugerencia editable. Definir un patrón de sufijo consistente en todo el explorador.

## Comparar detalles de los archivos en conflicto

**Categoría:** Resolución de conflictos  
**Prioridad:** ⚪ Opcional  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, macOS Finder

El diálogo de conflicto muestra datos comparables de ambos archivos (fecha de modificación, tamaño y miniatura/vista previa) para decidir con criterio cuál conservar.

**Comportamiento esperado:** Junto a cada versión se muestra tamaño, fecha de modificación y, si aplica, miniatura de imagen o icono de tipo. Debe resaltar diferencias útiles ('el del destino es más reciente'). Idealmente permitir seleccionar por casilla la versión de origen o de destino en conflictos múltiples. Casos borde: si los archivos son idénticos (mismo tamaño y fecha, o mismo hash), indicarlo y sugerir omitir; miniaturas no disponibles no deben bloquear ni retrasar el diálogo (marcador de posición mientras cargan).

> ℹ️ Windows 11 lo llama 'Comparar información de ambos archivos' con miniaturas y casillas por versión. Dolphin muestra detalles y previsualización de ambos. Muy valorado para no sobrescribir por error la versión buena.

## Combinar carpetas en conflicto (fusión recursiva)

**Categoría:** Resolución de conflictos  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files, macOS Finder

Al copiar/mover una carpeta cuyo nombre ya existe en el destino, fusionar su contenido de forma recursiva en vez de reemplazar la carpeta entera.

**Comportamiento esperado:** Cuando el conflicto es carpeta-contra-carpeta, ofrecer 'Combinar' (o 'Fusionar'): el contenido nuevo se añade dentro de la carpeta existente y SOLO los archivos que colisionan disparan el diálogo de conflicto de archivo (reemplazar/omitir/mantener ambos). Debe distinguirse claramente de 'Reemplazar', que NO debe borrar todo el contenido previo de la carpeta destino sin un aviso explícito. Casos borde: fusión profunda de árboles con conflictos a varios niveles debe respetar 'aplicar a todos'; permisos o archivos en uso a mitad de la fusión deben poder omitirse y reportarse al final; combinar una carpeta con un archivo del mismo nombre (tipo distinto) no es fusionable y debe tratarse como conflicto aparte.

> ℹ️ GROUNDING/AVISO: Nautilus/Nemo muestran 'Combinar' explícito para carpetas; Windows y Dolphin fusionan por defecto y solo preguntan por los archivos en conflicto. En macOS Finder 'Reemplazar' una carpeta la SUSTITUYE entera (borra lo anterior): Finder solo ofrece 'Combinar' si se mantiene Option (⌥) al soltar. Un explorador tipo Windows 11 debe fusionar por defecto y nunca reemplazar carpetas de forma destructiva sin advertir.

## Manejo de errores con reintentar / omitir / cancelar

**Categoría:** Errores y recuperación  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files, macOS Finder, Directory Opus, Total Commander

Cuando un archivo concreto falla (permiso denegado, archivo en uso, ruta demasiado larga, medio desconectado), mostrar un error claro que permita reintentar, omitir ese elemento o cancelar, sin abortar toda la operación por lotes.

**Comportamiento esperado:** Ante un fallo, el diálogo debe nombrar el archivo y el motivo en lenguaje llano ('El archivo está en uso por otra aplicación') y ofrecer 'Reintentar', 'Omitir' y 'Cancelar', más 'Omitir todos' / 'Reintentar todos' para no responder uno a uno. Omitir continúa con el resto; cancelar detiene de forma segura sin dejar estados a medias. Al finalizar, presentar un RESUMEN de lo omitido/fallido (lista de rutas) con opción de ver detalles o copiar el registro. Casos borde: 'destino sin espacio' debe pausar y ofrecer liberar espacio y reintentar en vez de fallar; 'acceso denegado' que requiera privilegios debe ofrecer reintentar con elevación/autenticación; un medio extraíble que se desconecta a mitad debe detectarse y avisar, no colgar la operación indefinidamente.

> ℹ️ Windows muestra 'Intentar de nuevo / Omitir / Cancelar' y errores tipo 'El archivo está en uso' con opción de reintentar. Nautilus/Dolphin ofrecen 'Omitir' / 'Omitir todos' / 'Reintentar'. La clave es no perder el trabajo ya hecho ni dejar la operación en estado ambiguo, y separar el feedback de error del de éxito.

## Enviar a la papelera

**Categoría:** Papelera y eliminación  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files, macOS Finder, Directory Opus, Total Commander

Al eliminar, el archivo va a una papelera/reciclaje reversible en lugar de borrarse de inmediato, permitiendo recuperación posterior.

**Comportamiento esperado:** La tecla Supr mueve la selección a la papelera. La operación es rápida (sin recorrer bytes salvo entre volúmenes) y debe registrar la ruta original y la fecha para poder restaurar. Casos borde: archivos en unidades externas/red pueden no tener papelera propia y deben ofrecer eliminación directa CON aviso; elementos mayores que la capacidad de la papelera deben avisar de que se borrarán permanentemente; la operación debe ser deshacible con Ctrl+Z.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Delete` | Enviar la selección a la papelera (Supr) en Windows/Dolphin/Nautilus/Nemo. macOS Finder: Cmd+Delete (la tecla Supr sola no hace nada en Finder) |

> ℹ️ En Linux la papelera sigue la especificación FreeDesktop (~/.local/share/Trash), con papeleras por volumen (.Trash-<uid>). Windows y Finder no piden confirmación por defecto al enviar a papelera; es configurable.

## Restaurar desde la papelera

**Categoría:** Papelera y eliminación  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files, macOS Finder

Devuelve un elemento de la papelera a su ubicación original con un clic, sin que el usuario tenga que recordar dónde estaba.

**Comportamiento esperado:** Desde la vista de papelera, 'Restaurar' recrea la ruta original (incluyendo carpetas intermedias si ya no existen) y coloca el archivo de vuelta. Debe mostrar la 'Ubicación original' y la 'Fecha de eliminación' como columnas. Casos borde: si ya existe un archivo con el mismo nombre en el destino, disparar el diálogo de conflicto normal; si la ubicación original está en una unidad desconectada, avisar; 'Restaurar todo' debe estar disponible.

> ℹ️ Windows muestra 'Ubicación original' y botón 'Restaurar' / 'Restaurar todos los elementos'. macOS Finder lo llama 'Devolver' (Put Back), sin atajo por defecto. Nautilus/Nemo/Dolphin muestran 'Restaurar' con la ruta original registrada.

## Eliminación permanente (Shift+Supr)

**Categoría:** Papelera y eliminación  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files, macOS Finder, Directory Opus, Total Commander

Borra la selección saltándose la papelera, para liberar espacio de inmediato o eliminar datos que no se quieren conservar.

**Comportamiento esperado:** Shift+Supr borra sin pasar por papelera y SIEMPRE debe pedir confirmación explícita ('¿Eliminar permanentemente estos N elementos? Esta acción no se puede deshacer'), porque no es reversible con Ctrl+Z. La confirmación debe indicar el número y, si es uno, el nombre. Casos borde: al eliminar de una unidad SIN papelera, el explorador cae de facto en eliminación permanente y debe advertirlo aunque se haya usado la tecla Supr normal; carpetas grandes deben mostrar diálogo de progreso y poder cancelarse.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Shift+Delete` | Eliminar permanentemente sin pasar por la papelera (Windows/Dolphin/Nautilus/Nemo). macOS Finder: Option+Cmd+Delete ('Eliminar inmediatamente') |

> ℹ️ Windows, Dolphin, Nautilus y Nemo usan Shift+Supr. macOS Finder usa Option+Cmd+Delete para 'Eliminar inmediatamente'. La confirmación es obligatoria por ser irreversible.

## Vaciar la papelera

**Categoría:** Papelera y eliminación  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files, macOS Finder

Acción para eliminar de forma permanente todo el contenido de la papelera y recuperar el espacio, con confirmación por su irreversibilidad.

**Comportamiento esperado:** Debe pedir confirmación indicando cuántos elementos y/o qué espacio se liberará. Ofrecer también 'eliminar de la papelera' elementos concretos sin vaciarla entera. Idealmente mostrar el tamaño ocupado por la papelera. Casos borde: si algún elemento está en uso o sin permisos, informar cuáles no se pudieron borrar y continuar con el resto; el vaciado debe poder cancelarse a mitad y mostrar progreso si es grande.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Shift+Cmd+Delete` | Vaciar papelera en macOS Finder (Option+Shift+Cmd+Delete la vacía sin confirmación). Windows/Linux normalmente no traen atajo por defecto; se hace desde el botón/menú |

> ℹ️ Windows: 'Vaciar la Papelera de reciclaje'. macOS Finder: 'Vaciar papelera' (Shift+Cmd+Delete; con Option añadido omite la confirmación). Dolphin/Nautilus/Nemo: 'Vaciar la papelera'. Es una eliminación permanente: la confirmación es la salvaguarda clave.

## Confirmación antes de eliminar (configurable)

**Categoría:** Papelera y eliminación  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files, macOS Finder

Diálogo opcional que pide confirmar antes de enviar a la papelera, ajustable por el usuario según prefiera seguridad o rapidez.

**Comportamiento esperado:** Debe existir un ajuste para pedir o no confirmación al enviar a papelera; la confirmación de eliminación PERMANENTE no debe poder desactivarse. El diálogo indica número de elementos y, con uno, su nombre; el foco por defecto debe estar en el botón seguro (Cancelar), no en Eliminar, para evitar borrados por Enter accidental. Casos borde: eliminar desde teclado con muchos elementos seleccionados debe mostrar el conteo para prevenir errores masivos.

> ℹ️ Windows trae la confirmación de papelera DESACTIVADA por defecto (activable en Propiedades de la Papelera de reciclaje). Nautilus/Nemo/Dolphin la ofrecen como preferencia. La de borrado permanente siempre debe mostrarse.

## Deshacer y rehacer operaciones de archivo

**Categoría:** Deshacer/Rehacer  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files, macOS Finder

Revierte (y vuelve a aplicar) la última operación de archivo: mover, copiar, renombrar, crear carpeta o enviar a papelera, sin tener que corregir a mano.

**Comportamiento esperado:** Ctrl+Z deshace la última acción reversible; Ctrl+Y (o Ctrl+Shift+Z) la rehace. Debe mantener una PILA de varias acciones, no solo una. Al deshacer 'enviar a papelera', el archivo vuelve a su sitio; al deshacer un movimiento, regresa al origen. Debe indicar qué se va a deshacer, idealmente en el propio menú ('Deshacer mover', 'Deshacer renombrar'). Casos borde: la eliminación permanente y el vaciado de papelera NO son deshacibles y deben marcarse como tal; si el estado cambió (el archivo ya no existe donde se dejó), avisar en vez de fallar en silencio; deshacer un renombrado debe recuperar exactamente el nombre previo.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+Z` | Deshacer la última operación de archivo. macOS Finder: Cmd+Z |
| `Ctrl+Y` | Rehacer (Windows). En GNOME Files/Nautilus, Nemo y Dolphin el rehacer es Ctrl+Shift+Z; macOS Finder: Cmd+Shift+Z |

> ℹ️ Windows usa Ctrl+Z / Ctrl+Y; los exploradores Linux y Finder usan Ctrl+Shift+Z / Cmd+Shift+Z para rehacer. Un explorador tipo Windows 11 en Linux debería aceptar AMBOS (Ctrl+Y y Ctrl+Shift+Z) para rehacer. La profundidad de la pila varía por explorador.

## Diálogo de propiedades (Alt+Enter)

**Categoría:** Propiedades y metadatos  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files, macOS Finder, Directory Opus, Total Commander

Ventana con la información detallada de un archivo o carpeta: tipo, tamaño, ubicación, fechas, atributos y accesos a permisos y aplicación por defecto.

**Comportamiento esperado:** Alt+Enter (o Alt+Doble clic) abre propiedades del elemento seleccionado. Muestra nombre, tipo, tamaño en disco vs. tamaño real, ubicación, fechas (creación/modificación/acceso) y atributos. Con selección múltiple agrega totales ('N archivos, M carpetas, tamaño total'). Debe organizarse en pestañas (General, Permisos, Abrir con, etc.). Casos borde: para carpetas debe iniciar el cálculo de tamaño en segundo plano; para accesos directos/enlaces mostrar el destino; para muchos elementos, no bloquear la UI mientras suma.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Alt+Enter` | Abrir propiedades del elemento seleccionado (Windows y Dolphin). GNOME Files/Nautilus y Nemo: Ctrl+I; macOS Finder: Cmd+I ('Obtener información') |

> ℹ️ Atajo Alt+Enter es el estándar Windows/Dolphin. Nautilus/Nemo usan Ctrl+I; Finder Cmd+I ('Obtener información'). Conviene soportar Alt+Enter por la estética Windows 11 objetivo.

## Cálculo del tamaño de carpeta

**Categoría:** Propiedades y metadatos  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files, macOS Finder, Directory Opus, Total Commander

Suma recursiva del contenido de una carpeta para conocer cuánto ocupa, mostrada en propiedades y opcionalmente en la propia columna de tamaño.

**Comportamiento esperado:** En propiedades, el tamaño se calcula en segundo plano y se actualiza en vivo ('Calculando… 1.234 archivos, 5,6 GB') hasta terminar, sin congelar el diálogo. Debe distinguir 'tamaño' de 'tamaño en disco' (bloques). Opción de mostrar tamaños de carpeta directamente en la vista de lista (como Dolphin y Directory Opus). Casos borde: enlaces simbólicos no deben contarse dos veces ni provocar bucles; carpetas sin permiso de lectura deben reflejar parciales y avisar; cancelar el cálculo debe ser posible; en árboles enormes, priorizar la responsividad de la UI.

> ℹ️ Windows y Finder calculan al abrir propiedades (Finder tiene 'Calcular todos los tamaños' en las opciones de vista). Dolphin y Directory Opus pueden mostrar tamaños de carpeta en línea en la lista, muy apreciado por usuarios avanzados. En Total Commander se calcula con la barra espaciadora sobre la carpeta o con Ctrl+Shift+Enter para todas.

## Gestión de permisos

**Categoría:** Propiedades y metadatos  
**Prioridad:** 🟡 Recomendable  
**Visto en:** KDE Dolphin, Cinnamon Nemo, GNOME Files, macOS Finder, Windows 11 File Explorer, Directory Opus

Pestaña de propiedades para ver y cambiar quién puede leer, escribir o ejecutar el archivo/carpeta (propietario/grupo/otros en Linux), y atributos como solo lectura.

**Comportamiento esperado:** Muestra permisos de forma comprensible (casillas Leer/Escribir/Ejecutar por propietario, grupo y otros) además de propietario y grupo. Permite aplicar recursivamente a carpetas ('aplicar a los elementos contenidos') con confirmación. Marca el atributo 'ejecutable' para scripts. Casos borde: cambios que requieren privilegios deben pedir autenticación (polkit/sudo) en vez de fallar silenciosamente; advertir al quitarse a uno mismo el permiso de escritura; en carpetas grandes, aplicar recursivo debe mostrar progreso y poder cancelarse.

> ℹ️ En Linux (Dolphin/Nautilus/Nemo) se exponen permisos POSIX rwx por propietario/grupo/otros. Windows usa una pestaña 'Seguridad' con ACLs y atributos (solo lectura/oculto). macOS Finder: 'Compartir y permisos'. Para un explorador Linux tipo Windows 11, el modelo POSIX es el relevante, presentado con estética moderna.

## Notificación de finalización

**Categoría:** Notificaciones y feedback  
**Prioridad:** ⚪ Opcional  
**Visto en:** GNOME Files, KDE Dolphin, Cinnamon Nemo, Directory Opus

Aviso al terminar una operación larga (sonido, notificación del sistema o cierre del diálogo con resumen), para no tener que vigilar el progreso.

**Comportamiento esperado:** Al completarse, si el diálogo estaba minimizado o en segundo plano, emitir una notificación del sistema con resumen ('Copia completada: 340 archivos'). Si hubo omisiones o errores, la notificación/resumen debe indicarlo y ofrecer 'Ver detalles'. Casos borde: no notificar operaciones triviales/instantáneas para evitar ruido; agrupar el resultado de varias operaciones cercanas; permitir silenciar sonidos; el feedback de error debe distinguirse claramente del de éxito (color/icono).

> ℹ️ GNOME Files/Nautilus y KDE/Plasma emiten notificaciones de fin cuando la operación corre en segundo plano. Windows cierra el diálogo (con sonido opcional) más que emitir una notificación. Un resumen de omitidos/errores al final es muy valorado.

