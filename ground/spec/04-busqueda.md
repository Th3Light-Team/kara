# 4. Búsqueda y filtrado

> Especificación de *conveniencias* (qué y cómo se comporta), agnóstica de implementación.

Especificación exhaustiva y fundamentada (grounded) de conveniencias de cara al usuario para el dominio "Búsqueda y filtrado" de un explorador de archivos con estética y comodidades tipo Windows 11 sobre Linux. Cubre la caja de búsqueda, la búsqueda incremental al escribir, el type-ahead find (saltar escribiendo por prefijo), el filtro en vivo por nombre, la selección de alcance (carpeta actual / subcarpetas / todo el equipo), los criterios por tipo, fecha, tamaño, contenido y etiquetas/valoración, el resaltado de coincidencias, la coincidencia sin distinción de mayúsculas/acentos, el historial y las búsquedas guardadas, el progreso y estado vacío, la apertura de la ubicación del resultado, la columna de ubicación en resultados, la sintaxis de consulta avanzada, los comodines/regex, el diálogo de búsqueda avanzada y el cierre/limpieza con Esc. Son 21 conveniencias, cada una fundamentada en al menos un explorador real (Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files/Nautilus, macOS Finder, Directory Opus y Total Commander), con atajos verificados y variantes anotadas por explorador. Correcciones de grounding aplicadas en esta revisión adversarial: (1) se retiró KDE Dolphin del type-ahead find porque su vista no ofrece salto por prefijo clásico al teclear; (2) se retiró Windows 11 del resaltado de coincidencias porque no enfatiza la subcadena coincidente en el nombre del resultado (grounding real en Total Commander y Directory Opus). El objetivo de prioridades es un explorador moderno tipo Windows: los "must" son la base imprescindible (buscar, saltar escribiendo, alcance, cerrar con Esc), los "should" elevan la comodidad al nivel esperado hoy, y los "could" son extras de power-user.

---

## Caja de búsqueda

**Categoría:** Entrada de búsqueda  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Total Commander

Campo de búsqueda que localiza archivos y carpetas por nombre dentro de la ubicación actual. Se invoca con un atajo o haciendo clic en el cuadro de la barra superior.

**Comportamiento esperado:** Al pulsar Ctrl+F (o hacer clic en el cuadro) el foco de teclado salta a la caja de búsqueda sin borrar la ruta actual ni la selección de archivos. En Windows 11 la caja vive en la esquina superior derecha y muestra un placeholder tipo 'Buscar en <carpeta>'. En cuanto se empieza a escribir, la barra de direcciones/migas cambia a un estado 'Resultados de búsqueda en <carpeta>' para dejar claro que la vista ya no es la carpeta plana. Casos borde: si no hay carpeta activa (p. ej. vista de dispositivos o 'Este equipo' sin ruta), la búsqueda debe deshabilitarse o ampliar el alcance a 'Este equipo'. El atajo debe funcionar aunque el foco esté en la lista de archivos o en el panel lateral. Volver a pulsar el atajo o hacer clic fuera no debe perder el término escrito.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+F` | Estándar en Windows 11, KDE Dolphin, Cinnamon Nemo y GNOME Files (Nautilus) para abrir/enfocar la búsqueda. |
| `Ctrl+E` | Windows 11 File Explorer: variante que también enfoca la caja de búsqueda. |
| `F3` | Windows 11 File Explorer: variante clásica que abre/enfoca la búsqueda. |
| `Cmd+F` | macOS Finder: inicia una búsqueda en la ventana/carpeta actual. |

> ℹ️ En Windows 11 conviven Ctrl+F, Ctrl+E y F3; para el objetivo tipo Windows, Ctrl+F es el atajo principal a documentar. No confundir con la barra de direcciones (Ctrl+L / Alt+D en Windows y Nautilus, Ctrl+L en Dolphin), que es navegación, no búsqueda.

## Resultados incrementales al escribir

**Categoría:** Entrada de búsqueda  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, GNOME Files (Nautilus), KDE Dolphin, macOS Finder

Los resultados se actualizan progresivamente conforme el usuario teclea en la caja de búsqueda, sin necesidad de pulsar Enter.

**Comportamiento esperado:** Cada carácter refina la lista de resultados con un pequeño retardo (debounce, ~150-300 ms) para no lanzar una búsqueda por pulsación en carpetas grandes o recursivas. Debe mostrarse un indicador de que la búsqueda sigue en curso mientras se recorren subcarpetas (ver 'Progreso y estado vacío'). Borrar caracteres re-amplía los resultados; vaciar la caja restaura la vista normal de la carpeta y su barra de direcciones. Pulsar Enter no es obligatorio, pero debe confirmar/forzar la búsqueda inmediata (saltándose el debounce) y, opcionalmente, mover el foco al primer resultado. Casos borde: búsquedas muy cortas (1 carácter) en árboles enormes pueden ser lentas, por lo que conviene mostrar primero las coincidencias de la carpeta actual y traer subcarpetas de forma progresiva.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Enter` | Confirma/fuerza la búsqueda inmediata y opcionalmente enfoca el primer resultado. |

> ℹ️ En redes o unidades sin índice la búsqueda incremental puede ser notablemente más lenta; el retardo y el progreso visible evitan la sensación de cuelgue. Es la búsqueda (recursiva/indexada), distinta del filtro en vivo por nombre y del type-ahead.

## Type-ahead find (saltar escribiendo)

**Categoría:** Navegación por teclado  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, Cinnamon Nemo, macOS Finder, Directory Opus, Total Commander

Escribir letras con el foco en la lista de archivos selecciona y salta al primer elemento cuyo nombre empieza por esas letras, sin abrir un cuadro de búsqueda ni recorrer subcarpetas.

**Comportamiento esperado:** Con el foco en la vista de archivos, teclear caracteres construye un buffer de prefijo que selecciona el primer archivo coincidente y hace scroll hasta él. El buffer se reinicia tras una pausa breve (~1 s) sin teclear. Escribir varias letras seguidas afina la coincidencia (p. ej. 'in','inf','info'); pulsar repetidamente la misma letra cicla entre los elementos que empiezan por ella. La coincidencia es por prefijo y sin distinción de mayúsculas. Debe convivir con la caja de búsqueda: type-ahead solo salta dentro de lo visible en la carpeta actual, nunca recurre en subcarpetas. Esc limpia el buffer y deshace el estado de coincidencia. Casos borde: números y símbolos deben aceptarse; el foco en un campo de texto (renombrado en línea, caja de búsqueda o filtro) no debe disparar type-ahead.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `(teclear letras)` | Sin modificador, con el foco en la lista de archivos: salta al primer nombre coincidente por prefijo. |
| `Esc` | Limpia el buffer de type-ahead y deshace el estado de coincidencia. |

> ℹ️ CORRECCIÓN DE GROUNDING: se retiró KDE Dolphin del seenIn. Dolphin usa su propia vista (KItemListView) que no hereda el type-ahead clásico de Qt, y al teclear no hace salto por prefijo por defecto (una carencia que sus usuarios reclaman de forma recurrente). GNOME Files (Nautilus) eliminó de forma polémica el type-ahead clásico y hace que teclear abra la búsqueda recursiva. Cinnamon Nemo conservó deliberadamente un comportamiento de salto/selección interactiva al teclear. En Total Commander el 'quick search' al teclear es configurable (solo letras, con Ctrl+Alt, etc.) y Directory Opus lo ofrece como 'find-as-you-type'. Para un explorador tipo Windows debe conservarse el type-ahead por prefijo como comportamiento por defecto.

## Filtro en vivo por nombre

**Categoría:** Filtrado en vivo  
**Prioridad:** 🟡 Recomendable  
**Visto en:** KDE Dolphin, Total Commander, Directory Opus

Barra de filtro que oculta al instante los elementos de la carpeta actual cuyo nombre no coincide con el texto escrito, sin recorrer subcarpetas ni depender de un índice.

**Comportamiento esperado:** A diferencia de la búsqueda, el filtro solo reduce lo ya listado en la carpeta actual y es inmediato (sin recursión ni indexado). Al escribir, los elementos no coincidentes desaparecen y el conteo de la barra de estado se actualiza ('N de M elementos'); al borrar el texto reaparecen todos. La coincidencia es por subcadena (contiene) y sin distinción de mayúsculas, idealmente con soporte de comodines. Debe indicarse visualmente que hay un filtro activo (barra visible, icono, conteo) para que el usuario no crea que la carpeta está vacía. Esc cierra la barra de filtro y restaura la vista. Casos borde: al navegar a otra carpeta, decidir si el filtro persiste o se limpia (Dolphin mantiene la barra visible pero suele reiniciar el texto); debe ser combinable con el orden y la agrupación activos.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+I` | KDE Dolphin: mostrar/ocultar la barra de filtro. |
| `Ctrl+S` | Total Commander: abre la búsqueda rápida con el filtro activado sobre el panel activo. |
| `Esc` | Cierra la barra de filtro y muestra de nuevo todos los elementos. |

> ℹ️ Windows 11 File Explorer no ofrece una barra de filtro en vivo separada (su caja es de búsqueda, que puede recurrir); GNOME Files y Finder tampoco tienen filtro no recursivo distinto. Es una comodidad muy valorada de Dolphin/Directory Opus/Total Commander que conviene incorporar y diferenciar claramente de la búsqueda recursiva. En Directory Opus la barra de filtro admite comodines e incluso patrones/expresiones para ocultar/mostrar.

## Alcance de búsqueda

**Categoría:** Alcance  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, GNOME Files (Nautilus), macOS Finder

Control para elegir dónde busca la consulta: solo la carpeta actual, la carpeta actual y todas sus subcarpetas, o todo el equipo/todas las ubicaciones.

**Comportamiento esperado:** Debe existir una forma clara de alternar entre 'Carpeta actual' y 'Todas las subcarpetas', más una opción para ampliar a 'Este equipo'/todas las ubicaciones. En Windows 11 la caja recurre en subcarpetas por defecto y el menú 'Opciones de búsqueda' permite acotar a 'Carpeta actual'. Cambiar el alcance debe reejecutar la búsqueda con el mismo término, sin obligar a reescribirlo. El alcance activo tiene que ser visible en todo momento (etiqueta o migas 'Resultados de búsqueda en <ámbito>'). Casos borde: la búsqueda en todo el equipo depende del índice del sistema y puede ser lenta o incompleta en rutas no indexadas; conviene avisar cuando una ubicación no está indexada y ofrecer una búsqueda profunda no indexada más lenta.

> ℹ️ Windows 11 usa 'Carpeta actual' / 'Todas las subcarpetas' (menú Opciones de búsqueda). Dolphin ofrece 'Desde aquí' vs 'En todas partes'. Finder ofrece la carpeta actual vs 'Este Mac'. Nautilus permite elegir buscar solo en la carpeta o en subcarpetas (y una preferencia global de alcance). Para el objetivo tipo Windows, el valor por defecto recomendado es recursivo en la carpeta actual con un toggle rápido a solo-carpeta.

## Buscar/filtrar por tipo

**Categoría:** Criterios de búsqueda  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, macOS Finder, KDE Dolphin, Directory Opus, Total Commander

Acotar los resultados por clase o extensión de archivo (documentos, imágenes, música, vídeo, carpetas, o una extensión concreta).

**Comportamiento esperado:** El usuario elige un tipo desde un menú/chips de filtros o lo escribe en la consulta (p. ej. 'kind:documento' o '*.pdf'). El filtro combina con el término de nombre y con otros criterios (fecha, tamaño). Debe reflejarse como un chip/etiqueta removible para poder quitarlo sin reescribir toda la consulta. Casos borde: las agrupaciones de tipo (p. ej. 'imágenes' cubre png/jpg/webp…) deben estar predefinidas de forma sensata; extensiones ambiguas o sin registrar deben tratarse por extensión literal; distinguir 'carpeta' de 'archivo' como tipo.

> ℹ️ Windows usa 'kind:' / 'type:' (Sintaxis de consulta avanzada, AQS) y chips en 'Opciones de búsqueda'. Finder ofrece el criterio 'Tipo/Clase'. Dolphin permite filtrar por tipo de archivo en las opciones de búsqueda (Baloo). Total Commander y Directory Opus filtran por máscara de extensión (p. ej. '*.jpg *.png'). Ofrecer tanto UI de chips como sintaxis textual cubre a usuarios casuales y avanzados.

## Buscar por fecha de modificación

**Categoría:** Criterios de búsqueda  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, macOS Finder, KDE Dolphin

Acotar resultados por fecha (modificado hoy, ayer, esta semana, este mes, este año, o un rango/fecha concretos).

**Comportamiento esperado:** Se ofrecen presets rápidos (Hoy, Ayer, Esta semana, El mes pasado, Este año) más la opción de fecha exacta o rango. El criterio combina con nombre, tipo y tamaño y aparece como chip removible. Debe quedar claro qué fecha se filtra (modificación por defecto; opcionalmente creación o último acceso). Casos borde: zonas horarias y límites de 'hoy'/'esta semana' deben calcularse en hora local; deben soportarse rangos abiertos ('después de X', 'antes de Y').

> ℹ️ Windows usa 'datemodified:' con presets y calendario en 'Opciones de búsqueda'. Finder y Dolphin ofrecen filtros de fecha equivalentes. Conviene exponer también fecha de creación como opción secundaria.

## Buscar por tamaño

**Categoría:** Criterios de búsqueda  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, macOS Finder, KDE Dolphin, Directory Opus

Acotar resultados por tamaño de archivo mediante rangos predefinidos o comparaciones numéricas.

**Comportamiento esperado:** Se ofrecen presets (Vacío, Diminuto, Pequeño, Mediano, Grande, Enorme, Gigantesco) y/o comparaciones explícitas (p. ej. '>100 MB', '<1 KB', un rango). Combinable con el resto de criterios y presentado como chip removible. Casos borde: unidades claras (KB/MB/GB en base binaria o decimal, a documentar); las carpetas no tienen tamaño calculado por defecto, por lo que el filtro de tamaño aplica a archivos.

> ℹ️ Windows usa 'size:' con presets (Empty/Tiny/Small/…) y operadores ('size:>100MB'). Finder ofrece el criterio de tamaño con mayor/menor que. Dolphin permite filtrar por tamaño en las opciones de búsqueda y Directory Opus en su búsqueda avanzada. Ofrecer comparadores numéricos además de presets es lo esperado por power-users.

## Búsqueda por contenido

**Categoría:** Criterios de búsqueda  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, macOS Finder, KDE Dolphin, Total Commander

Encontrar archivos por el texto que contienen en su interior, no solo por su nombre.

**Comportamiento esperado:** El usuario activa la búsqueda en contenido (opción o prefijo como 'content:') y el explorador busca dentro de archivos de texto y documentos indexados. Debe indicarse que puede ser más lenta y que depende del índice/formatos soportados. En resultados, idealmente se muestra un fragmento con la coincidencia. Casos borde: los archivos binarios se ignoran; sin índice, ofrecer una búsqueda por contenido bajo demanda (más lenta) acotada al alcance actual; respetar codificaciones de texto (UTF-8/Latin-1).

> ℹ️ Windows soporta 'content:' y contenido indexado. Finder busca en el contenido vía Spotlight. Dolphin puede buscar dentro de archivos (integración con Baloo). Total Commander ofrece 'Buscar texto' dentro de archivos en su diálogo Alt+F7 (no indexado, bajo demanda). Es una comodidad potente pero costosa; por defecto suele ir desactivada fuera de ubicaciones indexadas.

## Resaltar coincidencias

**Categoría:** Resultados  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Directory Opus, Total Commander

Destacar visualmente la parte del nombre (o del contenido) que coincide con el término buscado, para localizar el motivo de la coincidencia de un vistazo.

**Comportamiento esperado:** En los resultados o durante el filtro/type-ahead, la subcadena coincidente se resalta (negrita, subrayado o fondo de color) dentro del nombre del archivo. En búsqueda por contenido, resaltar el fragmento de texto que coincide. El resaltado debe respetar el tema claro/oscuro y mantener contraste suficiente. Casos borde: múltiples coincidencias en un mismo nombre deben resaltarse todas; con acentos/mayúsculas normalizados, el resaltado debe caer sobre los caracteres originales correctos.

> ℹ️ CORRECCIÓN DE GROUNDING: se retiró Windows 11 File Explorer del seenIn porque no resalta la subcadena coincidente en el nombre del resultado (solo selecciona la fila al saltar por escritura, sin enfatizar las letras). El grounding real es Total Commander (resalta las letras coincidentes en la búsqueda rápida) y Directory Opus (resalta coincidencias en filtro/búsqueda). Los exploradores mayoritarios (Windows, Finder, GNOME Files) son discretos aquí y no enfatizan la coincidencia. Es una comodidad muy valorada y barata que conviene incorporar aunque Windows no la tenga.

## Búsquedas recientes

**Categoría:** Historial  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, macOS Finder

Lista desplegable de términos buscados recientemente que se ofrece al enfocar la caja de búsqueda, para repetir consultas sin reescribirlas.

**Comportamiento esperado:** Al hacer clic o enfocar la caja vacía, aparece un desplegable con las últimas consultas; seleccionar una la reejecuta en el alcance actual. Debe poder navegarse con flechas y confirmarse con Enter, y ofrecer una forma de borrar el historial (privacidad). Casos borde: no mezclar términos de búsqueda con rutas de navegación; limitar el número de entradas; evitar duplicados actualizando el orden por recencia.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Flecha abajo / arriba` | Con el foco en la caja, recorre las búsquedas recientes del desplegable. |
| `Enter` | Reejecuta la búsqueda reciente seleccionada. |

> ℹ️ Windows 11 muestra el historial de búsquedas al abrir la caja (configurable/borrable por privacidad, en Opciones de carpeta). Finder muestra búsquedas recientes y guardadas en el menú del campo de búsqueda. Ofrecer un botón de 'borrar historial' es esperable por los usuarios sensibles a la privacidad.

## Búsquedas guardadas

**Categoría:** Historial  
**Prioridad:** ⚪ Opcional  
**Visto en:** Windows 11 File Explorer, macOS Finder, KDE Dolphin

Guardar una consulta (término + criterios + alcance) como un elemento reutilizable que se puede volver a abrir y que se re-evalúa al vuelo, tipo carpeta inteligente.

**Comportamiento esperado:** Tras construir una búsqueda, el usuario puede guardarla con un nombre; al reabrirla, se vuelve a ejecutar mostrando los resultados actuales (no una copia congelada). Debe accederse desde el panel lateral o un lugar de 'guardadas'. Casos borde: si la ubicación base ya no existe, avisar; permitir editar y borrar la búsqueda guardada; dejar claro que refleja el estado actual del disco.

> ℹ️ Windows permite 'Guardar búsqueda' (archivos .search-ms en la carpeta Búsquedas). Finder tiene 'Smart Folders' (carpetas inteligentes). Dolphin puede guardar búsquedas como entradas en 'Lugares'. Es un extra de power-user, no imprescindible para la primera versión.

## Limpiar/cerrar búsqueda con Esc

**Categoría:** Navegación por teclado  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Total Commander

Pulsar Esc vacía y cierra la búsqueda o el filtro activos y devuelve la vista normal de la carpeta.

**Comportamiento esperado:** Con el foco en la caja de búsqueda o filtro, Esc borra el término y cierra la barra, restaurando la lista completa de la carpeta y el estado previo de la barra de direcciones/migas. Un primer Esc podría solo limpiar el texto y un segundo cerrar la barra (comportamiento escalonado, opcional pero cómodo). Debe devolver el foco a la lista de archivos. Casos borde: si hay resultados recursivos mostrados, Esc también debe volver a la carpeta base; no debe afectar a otras selecciones ni deshacer operaciones de archivo.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Esc` | En caja de búsqueda o barra de filtro: limpia el término y cierra la barra, restaurando la vista. |
| `Cmd+.` | macOS Finder: cancela/cierra la búsqueda en curso (equivalente a Esc). |

> ℹ️ Comportamiento universal y esperado. En macOS también se usa Cmd+. para cancelar. Considerar el Esc escalonado (limpiar texto → cerrar barra) por ergonomía, como hacen varios exploradores.

## Abrir la ubicación del resultado

**Categoría:** Resultados  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, macOS Finder, GNOME Files (Nautilus), KDE Dolphin, Cinnamon Nemo, Directory Opus, Total Commander

Desde un resultado de búsqueda, saltar a la carpeta que contiene el elemento (y seleccionarlo allí) para operar sobre él en su ubicación real.

**Comportamiento esperado:** Con un resultado seleccionado, una acción de menú contextual abre la carpeta contenedora y deja el archivo seleccionado y visible. Un doble clic en un resultado de tipo carpeta navega dentro; un doble clic en un archivo lo abre con su aplicación asociada, por lo que 'abrir ubicación' debe ser una acción distinta y explícita. Debe funcionar tanto en resultados recursivos como en búsqueda de todo el equipo. Casos borde: si el elemento fue movido/eliminado durante la búsqueda, avisar en lugar de fallar en silencio; con varios elementos seleccionados, la acción aplica al elemento con foco.

> ℹ️ NUEVA. Es una comodidad esencial y universal, ausente del borrador. Windows: menú contextual 'Abrir ubicación del archivo'. Finder: 'Mostrar en la carpeta que lo contiene' (Show in Enclosing Folder). Nautilus: 'Abrir la ubicación del elemento' (Open Item Location). Dolphin/Nemo lo ofrecen en el menú contextual de los resultados. No suele tener un atajo por defecto universal; documentar la acción de menú es suficiente.

## Ubicación/ruta en los resultados

**Categoría:** Resultados  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, macOS Finder, GNOME Files (Nautilus), KDE Dolphin

Mostrar la carpeta contenedora (ruta) de cada resultado de búsqueda para distinguir archivos con el mismo nombre en distintas carpetas.

**Comportamiento esperado:** En búsquedas recursivas o de todo el equipo, cada fila debe indicar su ubicación: una columna 'Carpeta'/'Ruta' en la vista de detalles, o un subtítulo bajo el nombre. La ubicación debe actualizarse conforme llegan resultados progresivamente y debe poder ordenarse por ella. Casos borde: rutas muy largas se recortan con puntos suspensivos pero deben verse completas en tooltip; no mostrar la columna cuando la búsqueda es solo de la carpeta actual (todo comparte ubicación).

> ℹ️ NUEVA. Windows añade automáticamente la columna 'Carpeta' en los resultados de búsqueda. Finder muestra la ruta en la barra inferior al seleccionar y permite añadir la columna 'Ruta'. Nautilus muestra la ubicación como subtítulo del resultado. Dolphin muestra la ruta relativa en los resultados de búsqueda. Sin esta pista, los resultados recursivos son ambiguos.

## Sintaxis de consulta avanzada

**Categoría:** Sintaxis avanzada  
**Prioridad:** ⚪ Opcional  
**Visto en:** Windows 11 File Explorer, macOS Finder, Total Commander, Directory Opus

Operadores y filtros escribibles en la propia caja para consultas precisas (por propiedad, comparadores y booleanos).

**Comportamiento esperado:** La caja acepta filtros con prefijo de propiedad y comparadores, p. ej. 'name:', 'ext:', 'size:>1GB', 'datemodified:esta semana', combinables con operadores booleanos 'AND'/'OR'/'NOT' y comillas para frases exactas. Debe ser tolerante: el texto sin operadores se trata como búsqueda por nombre/contenido normal. Idealmente ofrecer autocompletado de propiedades. Casos borde: los errores de sintaxis no deben romper la búsqueda (degradar a coincidencia textual); documentar las palabras clave localizadas y sus equivalentes en inglés.

> ℹ️ Windows usa AQS ('kind:', 'size:>', 'ext:', 'AND/OR/NOT'). Finder admite consultas raw/atributos. Total Commander y Directory Opus tienen sus propios lenguajes de búsqueda. Es una comodidad de power-user; conviene que la UI de chips genere internamente esta sintaxis para coherencia.

## Comodines y patrones

**Categoría:** Sintaxis avanzada  
**Prioridad:** ⚪ Opcional  
**Visto en:** KDE Dolphin, Total Commander, Directory Opus, Windows 11 File Explorer

Soporte de comodines (* para cualquier secuencia, ? para un carácter) y, en herramientas avanzadas, expresiones regulares, en la búsqueda y en el filtro por nombre.

**Comportamiento esperado:** El usuario puede escribir patrones como '*.txt', 'informe_??.pdf' o 'foto*' tanto en la búsqueda como en la barra de filtro en vivo. La coincidencia es sin distinción de mayúsculas por defecto. Debe convivir con la búsqueda por subcadena simple (sin comodines, 'contiene'). En herramientas de power-user debe poder activarse coincidencia por expresión regular como modo alternativo. Casos borde: escapar literales '*' y '?'; decidir si un patrón sin comodines equivale a 'contiene' o a 'empieza por'; soportar listas de varios patrones separados (p. ej. '*.jpg *.png').

> ℹ️ Total Commander y Directory Opus tienen soporte de comodines (y listas de patrones) muy completo, además de expresiones regulares; la barra de filtro de Dolphin también admite comodines. Windows los acepta parcialmente en la caja. Documentar bien la semántica de '*'/'?' y el modo regex evita sorpresas.

## Progreso y estado vacío de la búsqueda

**Categoría:** Resultados  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, GNOME Files (Nautilus), KDE Dolphin, macOS Finder

Retroalimentación clara del estado de la búsqueda: indicador de progreso mientras recorre, conteo de resultados y un estado vacío explícito cuando no hay coincidencias.

**Comportamiento esperado:** Mientras la búsqueda recursiva o por contenido está en curso, mostrar un indicador (barra o spinner) y, si es posible, resultados que van llegando de forma progresiva. Al terminar, mostrar el número de coincidencias en la barra de estado. Si no hay resultados, presentar un estado vacío explícito ('No se encontraron elementos que coincidan con <término>') con sugerencias (ampliar alcance, revisar filtros). Casos borde: permitir cancelar una búsqueda larga con Esc; si el alcance no está indexado, avisar de que puede tardar; no dejar la vista ambigua entre 'buscando' y 'sin resultados'.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Esc` | Cancela una búsqueda en curso además de cerrar la caja. |

> ℹ️ Windows muestra una barra de progreso verde en la barra de direcciones durante búsquedas largas. Diferenciar claramente 'buscando…' de 'sin resultados' es clave para no confundir al usuario con una lista vacía prematura.

## Coincidencia sin distinción de mayúsculas ni acentos

**Categoría:** Resultados  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Total Commander

La búsqueda, el filtro y el type-ahead coinciden por defecto sin importar mayúsculas/minúsculas y, preferiblemente, plegando acentos y diacríticos.

**Comportamiento esperado:** Escribir 'informe' encuentra 'Informe', 'INFORME' e idealmente 'informé'. El plegado de acentos debe ser opcionalmente desactivable para usuarios que necesiten precisión. Debe aplicarse de forma consistente en la caja de búsqueda, la barra de filtro en vivo y el type-ahead. Casos borde: alfabetos no latinos y normalización Unicode (NFC/NFD); mantener el resaltado sobre los caracteres originales aunque la comparación se haya normalizado.

> ℹ️ El comportamiento sin distinción de mayúsculas es universal; el plegado de acentos varía entre exploradores. Para un explorador en español es muy valorado que 'accion' encuentre 'acción'. Considerar una opción para distinguir mayúsculas/acentos en búsquedas exigentes.

## Buscar por etiquetas y valoración

**Categoría:** Criterios de búsqueda  
**Prioridad:** ⚪ Opcional  
**Visto en:** macOS Finder, KDE Dolphin, Windows 11 File Explorer

Acotar resultados por metadatos definidos por el usuario: etiquetas/colores y valoración por estrellas.

**Comportamiento esperado:** El usuario filtra por una o varias etiquetas (o color de etiqueta) y/o por valoración (p. ej. '>=4 estrellas'), combinable con nombre, tipo, fecha y tamaño, y presentado como chip removible. Debe convivir con el resto de criterios. Casos borde: no todos los formatos guardan estas etiquetas de forma nativa (a veces se almacenan en metadatos del sistema/índice), por lo que debe indicarse su alcance; ofrecer autocompletado de etiquetas existentes.

> ℹ️ NUEVA. Finder ofrece etiquetas/colores (Tags) como criterio de búsqueda de primera clase. Dolphin, vía Baloo, permite buscar por etiquetas y por valoración (rating). Windows admite buscar por 'tags:' en tipos de archivo que soportan la propiedad. Es un extra de power-user útil para quien organiza con metadatos.

## Diálogo/panel de búsqueda avanzada

**Categoría:** Sintaxis avanzada  
**Prioridad:** ⚪ Opcional  
**Visto en:** Total Commander, Directory Opus, Windows 11 File Explorer

Ventana o panel dedicado que reúne todos los criterios (nombre, ubicación, tipo, fecha, tamaño, contenido, atributos) en un formulario, con plantillas guardables y opciones que la caja en línea no expone.

**Comportamiento esperado:** El usuario abre un diálogo/panel donde compone la búsqueda con campos separados, elige el punto de partida, activa la búsqueda dentro de archivos y (en algunos exploradores) dentro de archivos comprimidos, y puede guardar/cargar el conjunto de criterios como plantilla. El resultado se muestra como una lista navegable desde la que se puede saltar a la ubicación de cada elemento. Casos borde: búsquedas largas deben poder cancelarse; ofrecer 'buscar en resultados' para refinar sobre el conjunto ya encontrado.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Alt+F7` | Total Commander: abre el diálogo de búsqueda de archivos. |

> ℹ️ NUEVA. Total Commander (Alt+F7) y Directory Opus (Find Files / búsqueda avanzada) ofrecen paneles completos con criterios combinados, búsqueda en contenido y en archivos comprimidos, y plantillas reutilizables. Windows históricamente tuvo un panel de búsqueda avanzada; hoy expone criterios vía chips y AQS. Es un extra de power-user complementario a la caja en línea.

