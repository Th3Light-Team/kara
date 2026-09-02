# 1. Navegación e historial

> Especificación de *conveniencias* (qué y cómo se comporta), agnóstica de implementación.

Especificación exhaustiva y fundamentada (grounded) de conveniencias de navegación e historial para un explorador de archivos con estética y comodidades tipo Windows 11 sobre Linux. Cubre la barra de direcciones/breadcrumb editable, atrás/adelante/subir, historial, refresco, panel de árbol, navegación con teclado, Acceso rápido/marcadores, pestañas, paneles duales, copiar ruta, restaurar sesión y gestos de navegación. Cada conveniencia está fundamentada en exploradores reales (Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files/Nautilus, macOS Finder, Total Commander, Directory Opus) con atajos verificados y sus variantes/conflictos anotados. Esta versión es una revisión adversarial: corrige atribuciones erróneas (p.ej. Ctrl+D, Alt+Home, historial de Finder, reabrir pestaña), acota atajos que son convención de navegador y no garantizados, y añade conveniencias que faltaban (refrescar, navegación con teclado, copiar ruta, restaurar sesión).

---

## Barra de direcciones tipo breadcrumb (migas de pan)

**Categoría:** Barra de direcciones  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus)

La ruta actual se muestra como una secuencia de segmentos clicables (Este equipo > Documentos > Proyectos) en vez de texto plano. Cada segmento navega a esa carpeta ancestro con un solo clic.

**Comportamiento esperado:** Clic en cualquier segmento navega a esa carpeta ancestro. La barra refleja siempre la ubicación actual y se actualiza al navegar por otros medios (árbol, atrás/adelante, doble clic, escribir ruta). Si la ruta es más larga que el ancho disponible, los segmentos iniciales se colapsan en un botón de desbordamiento (« o chevrons) que despliega los ancestros ocultos en un menú. Al hacer clic en la zona vacía de la barra (a la derecha de las migas) o pulsar Ctrl+L cambia a modo edición de texto con la ruta completa seleccionada; Esc vuelve a migas sin cambiar de carpeta. Debe truncar por el centro/inicio, nunca ocultar el segmento actual (el último), y mantener siempre visible al menos la carpeta actual y su padre. Casos borde: rutas de red y ubicaciones especiales (Papelera, Este equipo) también deben representarse como segmentos coherentes.

> ℹ️ Finder no usa migas en la barra superior; muestra una 'barra de ruta' clicable opcional en la parte inferior (Ver > Mostrar barra de ruta). En Dolphin/Nemo un botón alterna de forma permanente entre migas y campo de texto. Windows separa cada segmento con una flecha desplegable (ver 'Desplegables de hermanos').

## Desplegables de hermanos entre segmentos

**Categoría:** Barra de direcciones  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer

Entre cada par de segmentos de las migas hay una pequeña flecha que, al pulsarla, despliega las subcarpetas hermanas de ese nivel para saltar lateralmente sin retroceder.

**Comportamiento esperado:** Clic en la flecha situada a la derecha de un segmento abre un menú con las carpetas contenidas en ese ancestro; elegir una navega directamente. La subcarpeta por la que se pasa actualmente suele mostrarse marcada (con un check o resaltada). El menú debe ser desplazable si hay muchas entradas, abrirse también con teclado (foco en la flecha + Enter/flecha abajo), y cerrarse con Esc. Permite moverse entre carpetas hermanas en un único gesto sin pasar por la carpeta padre.

> ℹ️ Es una seña de identidad del Explorador de Windows y muy útil para replicar su 'sensación'. Dolphin/Nemo ofrecen una variante distinta: al hacer clic en un segmento de migas aparece un desplegable de subcarpetas de ESE segmento (no una flecha intermedia entre segmentos). Finder y Nautilus no lo tienen de serie.

## Editar y escribir ruta (Ctrl+L)

**Categoría:** Barra de direcciones  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder

Alternar la barra de direcciones a un campo de texto editable para escribir o pegar una ruta completa (o ubicación de red) y saltar directamente a ella.

**Comportamiento esperado:** Ctrl+L (o clic en la zona vacía de la barra) convierte las migas en un campo de texto con la ruta actual seleccionada para sobrescribir. Enter navega a la ruta escrita; Esc cancela y vuelve a migas sin moverse. Debe aceptar pegar una ruta y navegar, admitir rutas absolutas, ~ para Inicio (en Linux), variables de entorno ($HOME) y rutas de red (UNC \\servidor\recurso al estilo Windows, smb:// / sftp:// en Linux). Si la ruta no existe o es un archivo, mostrar error sin borrar lo escrito ni cambiar de carpeta. Autofoco con todo el texto seleccionado para reescribir rápido. Si la ruta apunta a un archivo, opción de abrirlo o de ir a su carpeta contenedora.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+L` | Enfocar y editar la barra de direcciones como texto (estándar de facto en Linux y también válido en Windows) |
| `F4` | Windows — enfoca la barra y despliega el historial de rutas escritas |
| `Alt+D` | Windows — alternativa para enfocar la barra de direcciones |
| `F6` | KDE Dolphin — enfocar/alternar la barra de ubicación (convención KDE) |
| `Cmd+Shift+G` | macOS Finder — diálogo 'Ir a la carpeta' |

> ℹ️ En Finder no es la barra superior sino el diálogo 'Ir a la carpeta' (Cmd+Shift+G) con autocompletado. Dolphin/Nemo tienen un botón que alterna de forma permanente entre migas y campo. F4 en Windows además abre el desplegable de ubicaciones anteriores. Ctrl+L, Alt+D y F4 funcionan todos en Windows para enfocar la barra.

## Autocompletado de rutas

**Categoría:** Barra de direcciones  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, GNOME Files (Nautilus), macOS Finder

Al escribir una ruta en la barra de direcciones, el explorador sugiere y completa nombres de carpetas existentes que coinciden con lo tecleado.

**Comportamiento esperado:** Mientras se escribe aparece una lista desplegable de coincidencias (carpetas hijas que empiezan por el texto) y/o autocompletado en línea del resto del nombre. Tab o las flechas recorren las sugerencias; Enter acepta y navega; Esc cierra la lista sin perder lo escrito. Debe completar segmento a segmento: al escribir el separador ('/' en Linux, '\' al estilo Windows) ofrece el contenido del nuevo nivel. Respeta la sensibilidad a mayúsculas del sistema de archivos subyacente. Conviene mezclar rutas del historial de rutas escritas además de las del disco, pero SIN bloquear la escritura de una ruta que aún no existe (el autocompletado no debe imponer una sugerencia si el usuario sigue tecleando).

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Tab` | Recorrer/aceptar la sugerencia de autocompletado |
| `Flecha abajo` | Abrir/recorrer la lista de sugerencias |

> ℹ️ Nautilus autocompleta en el campo de Ctrl+L; Finder en el diálogo Cmd+Shift+G. Windows mezcla sugerencias del historial escrito con las del disco. Evitar un autocompletado agresivo que impida teclear una ruta nueva o borrar hacia atrás.

## Atrás y Adelante

**Categoría:** Historial de navegación  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Total Commander, Directory Opus

Botones y atajos para retroceder y avanzar por las ubicaciones visitadas dentro de la ventana/pestaña, igual que en un navegador web.

**Comportamiento esperado:** 'Atrás' vuelve a la carpeta vista anteriormente en la pila de historial; 'Adelante' rehace ese movimiento (solo disponible tras haber ido atrás). Navegar a una carpeta nueva después de retroceder trunca la pila de 'adelante'. Al volver, se restaura la selección y la posición de desplazamiento (scroll) anteriores de esa ubicación. Los botones se deshabilitan visualmente (atenuados) cuando no hay historial en esa dirección. Deben responder también a los botones laterales del ratón (XButton1/XButton2). El historial es independiente por pestaña. Caso borde: si una carpeta del historial deja de existir, saltar a la más cercana disponible o marcar la entrada como no válida.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Alt+Left` | Atrás |
| `Alt+Right` | Adelante |
| `Backspace` | Windows — Atrás (carpeta anterior del historial); ver conflicto con 'Subir' |
| `XButton1 / XButton2` | Botones laterales del ratón: atrás / adelante |
| `Cmd+[ / Cmd+]` | macOS Finder — atrás / adelante |

> ℹ️ Conflicto clásico: en Windows la tecla Backspace hace 'Atrás', mientras que en varios gestores Linux (KDE/comandantes) y en el hábito clásico hace 'Subir'. Hay que elegir una convención y ser coherente (ver 'Subir un nivel') y dejarlo configurable. Restaurar el scroll/selección al volver es lo que distingue una buena implementación.

## Desplegable de historial de navegación

**Categoría:** Historial de navegación  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, Total Commander, Directory Opus, KDE Dolphin

Menú que lista las ubicaciones visitadas recientemente para saltar a cualquiera de ellas sin pulsar Atrás repetidamente.

**Comportamiento esperado:** Mantener pulsado (o hacer clic en una flechita adjunta) sobre los botones Atrás/Adelante despliega la lista ordenada de ubicaciones del historial de la pestaña; elegir una salta directamente a ese punto. La entrada de la ubicación actual suele resaltarse. En Windows existe además un historial global de rutas escritas en la barra de direcciones (F4). Debe distinguirse el historial atrás/adelante de la pestaña del historial de rutas tecleadas.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `F4` | Windows — desplegar ubicaciones anteriores (rutas escritas) en la barra de direcciones |
| `Alt+Down` | Total Commander / Directory Opus — abrir el historial de carpetas del panel activo |

> ℹ️ CORREGIDO: en macOS Finder los botones atrás/adelante NO despliegan un menú de historial al mantenerlos pulsados; el historial reciente se expone en el menú Ir > Carpetas recientes. En Windows el desplegable está en el botón junto a Atrás/Adelante y en la flecha de la barra (F4). Dolphin muestra el historial reciente en el menú 'Ir'. Los comandantes usan Alt+Flecha abajo para el historial del panel activo.

## Subir un nivel (a la carpeta padre)

**Categoría:** Navegación jerárquica  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Total Commander, Directory Opus

Ir a la carpeta contenedora de la actual, con independencia del historial de atrás/adelante.

**Comportamiento esperado:** 'Subir' navega siempre a la carpeta padre en la jerarquía, aunque no se hubiera visitado antes (a diferencia de 'Atrás', que sigue la pila del historial). Al subir se deja seleccionada/resaltada y con scroll visible la carpeta de la que se venía, para orientarse. El botón/atajo se deshabilita en la raíz del sistema de archivos o de la unidad. Un botón dedicado (flecha arriba) acompaña al atajo de teclado.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Alt+Up` | Subir un nivel (estándar recomendado para el proyecto; también válido en Windows, Dolphin, Nautilus) |
| `Backspace` | KDE Dolphin / Total Commander / Nautilus clásico — subir (OJO: en Windows Backspace es 'Atrás') |
| `Cmd+Up` | macOS Finder — carpeta contenedora |

> ℹ️ Conflicto histórico de Backspace: significa 'Atrás' en Windows pero 'Subir' en la convención KDE/comandantes y en el hábito clásico (Nautilus antiguo; en GNOME Files moderno Backspace puede no estar mapeado). Al ser un proyecto de estética Windows 11, usar Alt+Up para subir y reservar Backspace para 'Atrás', dejándolo configurable. Alt+Up es un atajo seguro y universal para 'Subir' en Windows y Linux.

## Ir a Inicio

**Categoría:** Navegación jerárquica  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder

Acceso directo a la ubicación de inicio: en Windows 11 la vista 'Inicio' (con acceso rápido, favoritos y recientes); en Linux/macOS la carpeta personal del usuario.

**Comportamiento esperado:** Un clic (o Alt+Home) lleva a la ubicación de inicio configurada. En un explorador tipo Windows 11, 'Inicio' es una vista agregada (fijados + frecuentes + recientes) fijada en lo alto del panel lateral; conviene ofrecer además un acceso directo a la carpeta personal (~). Debe ser la ubicación por defecto al abrir una ventana o pestaña nueva, y ser configurable (Inicio agregado vs. carpeta personal vs. última ubicación).

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Alt+Home` | GNOME Files / Dolphin / Nemo — ir a la carpeta personal (~); convención de Linux adoptada para el proyecto |
| `Cmd+Shift+H` | macOS Finder — carpeta de inicio del usuario |

> ℹ️ CORREGIDO: Alt+Home NO es un atajo estándar del Explorador de Windows 11 (allí 'Inicio' se alcanza desde el panel lateral); Alt+Home es la convención de Nautilus/Dolphin/Nemo para ir a ~. El proyecto la adopta como diseño. Distinguir 'Inicio' como vista agregada (Windows 11) de 'Carpeta personal' (~). Muchos usuarios esperan que Alt+Home vaya directamente a ~. Ofrecer una opción para elegir qué abre una ventana/pestaña nueva.

## Panel de navegación en árbol

**Categoría:** Panel de navegación  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), Directory Opus

Barra lateral izquierda que muestra la jerarquía de carpetas y ubicaciones (unidades, red, favoritos) para navegar y ver el contexto de la ubicación actual.

**Comportamiento esperado:** Clic en un nodo navega a esa carpeta en el panel principal. Los nodos con subcarpetas muestran un triángulo/flecha para expandir sin navegar. El panel se puede mostrar/ocultar y redimensionar arrastrando su borde (con anchura persistente entre sesiones). Se organiza en secciones (Acceso rápido/Favoritos, Este equipo/Unidades, Dispositivos extraíbles, Red). Debe admitir arrastrar y soltar archivos sobre nodos del árbol para mover/copiar (con feedback del destino), y menú contextual por nodo (abrir en pestaña/ventana nueva, anclar, expandir todo, etc.).

> ℹ️ CORREGIDO/MATIZADO: la barra lateral de macOS Finder es una lista PLANA de favoritos/ubicaciones, no un árbol expandible; la expansión jerárquica en Finder aparece solo en la vista de lista del panel principal (triángulos de despliegue), por eso se retira Finder de este seenIn del 'árbol de la barra lateral'. Nautilus moderno muestra 'lugares' (marcadores) planos en la barra lateral, con árbol expandible dentro de la vista de lista. Dolphin ofrece un panel de 'Carpetas' en árbol aparte del de 'Lugares'. Windows combina árbol y accesos en un solo panel. El panel se activa/desactiva desde el menú Ver.

## Expandir y colapsar nodos del árbol

**Categoría:** Panel de navegación  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, macOS Finder, Directory Opus

Abrir y cerrar ramas del árbol de carpetas con los triángulos/flechas o el teclado, sin cambiar necesariamente la carpeta mostrada en el panel principal.

**Comportamiento esperado:** Clic en el triángulo expande/colapsa la rama mostrando sus subcarpetas. Con el foco en un nodo, Flecha derecha expande (o baja al primer hijo si ya está expandido) y Flecha izquierda colapsa (o sube al padre si ya está colapsado). En Windows, el teclado numérico * expande recursivamente toda la subrama; + expande y − colapsa el nodo actual. Expandir un nodo NO debe forzar la navegación del panel principal. Debe usar carga diferida: no leer el contenido de una rama hasta que se expande.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Flecha derecha` | Expandir el nodo / entrar en la rama, con el foco en el árbol |
| `Flecha izquierda` | Colapsar el nodo / subir al padre |
| `Numpad *` | Windows — expandir recursivamente toda la subrama del nodo con foco |
| `Numpad + / Numpad −` | Windows — expandir / colapsar el nodo actual |

> ℹ️ El asterisco del teclado numérico para 'expandir todo' es muy apreciado por usuarios avanzados (Windows). En Finder/Dolphin, Flecha derecha/izquierda expande/colapsa en la vista de lista/árbol. Cuidar el rendimiento y la carga diferida al expandir ramas con muchísimas subcarpetas; ofrecer un límite o aviso al 'expandir todo' sobre árboles enormes.

## Sincronizar el árbol con la carpeta actual

**Categoría:** Panel de navegación  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, macOS Finder, Directory Opus

El árbol de navegación se expande y resalta automáticamente para reflejar la carpeta que se está viendo en el panel principal.

**Comportamiento esperado:** Al navegar por cualquier medio (migas, atrás/adelante, doble clic, escribir ruta), el árbol despliega los ancestros necesarios, resalta el nodo de la carpeta actual y lo hace visible mediante auto-desplazamiento. Es una opción activable ('Expandir para abrir la carpeta actual'). No debe colapsar ramas que el usuario abrió manualmente si no es necesario, ni provocar 'saltos' bruscos de scroll al navegar rápido (conviene animar o diferir el auto-scroll). Con la opción desactivada, el árbol permanece estático salvo interacción directa.

> ℹ️ En Windows es la opción 'Expandir para abrir la carpeta actual' de Opciones de carpeta (desactivada por defecto en Windows 10/11). Muy pedida para no perder el contexto de dónde se está. Ofrecerla como interruptor en Ajustes.

## Acceso rápido / Lugares con carpetas fijadas

**Categoría:** Marcadores y accesos  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus

Sección del panel lateral donde el usuario fija (ancla) carpetas favoritas para acceder a ellas con un clic desde cualquier ubicación.

**Comportamiento esperado:** Arrastrar una carpeta al área de Acceso rápido/Favoritos, o usar 'Anclar a Acceso rápido' / 'Añadir a marcadores' en el menú contextual, la fija. Las entradas fijadas se reordenan arrastrando y se quitan desde su menú contextual. Persisten entre sesiones. Un icono de pin distingue las fijadas de las frecuentes automáticas. Clic navega; deberían admitir también abrir en pestaña nueva (clic central) y renombrar el alias sin renombrar la carpeta real. Caso borde: una entrada cuyo destino ya no existe debe marcarse como no disponible en vez de fallar silenciosamente.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+D` | GNOME Files (Nautilus) / Cinnamon Nemo — añadir la carpeta actual a marcadores/lugares |

> ℹ️ CORREGIDO: Ctrl+D para 'añadir marcador' es de Nautilus/Nemo (no de Dolphin, que añade a 'Lugares' por arrastre o menú contextual sin atajo estándar). Nota: en Total Commander Ctrl+D abre la lista de directorios favoritos (hotlist), un significado distinto. Los nombres varían: 'Acceso rápido' (Windows), 'Lugares'/'Marcadores' (Dolphin/Nautilus/Nemo), 'Favoritos' (Finder). Finder permite un alias distinto del nombre real de la carpeta. Conviene poder crear separadores/secciones para organizar muchos accesos.

## Ubicaciones frecuentes y archivos recientes

**Categoría:** Marcadores y accesos  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, GNOME Files (Nautilus), macOS Finder

Listas que se rellenan automáticamente con las carpetas más usadas y los archivos abiertos recientemente, para volver a ellos sin buscarlos.

**Comportamiento esperado:** La vista de Inicio/Acceso rápido muestra 'Carpetas frecuentes' y 'Recientes' generados de forma automática según el uso. Clic abre la carpeta o el archivo. Debe permitir quitar una entrada concreta ('Quitar de recientes'), desactivar por completo el seguimiento por privacidad, y una acción para borrar todo el historial reciente. Se distingue visualmente de las carpetas fijadas manualmente (sin icono de pin). Caso borde: no listar archivos/carpetas que ya no existen o de volúmenes desmontados.

> ℹ️ Sensible a la privacidad: ofrecer un interruptor para no registrar el historial y un 'Borrar recientes'. Windows lo integra en la vista Inicio; Nautilus y Finder tienen un lugar 'Recientes' dedicado en la barra lateral. En Linux, respetar el estándar recently-used.xbel de freedesktop cuando aplique.

## Pestañas de carpetas

**Categoría:** Pestañas y paneles  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Total Commander

Varias carpetas abiertas en pestañas dentro de una misma ventana, cada una con su propio historial de navegación.

**Comportamiento esperado:** Ctrl+T abre una pestaña nueva (en la carpeta actual o en Inicio, configurable); Ctrl+W la cierra. Se recorren con Ctrl+Tab / Ctrl+Shift+Tab y se salta directamente con Ctrl+1…Ctrl+8. Las pestañas se reordenan arrastrando y se puede arrastrar una carpeta sobre una pestaña para abrirla ahí. Cada pestaña conserva su historial atrás/adelante, su ruta y su selección de forma independiente. Un botón '+' añade pestañas; el clic central sobre una pestaña la cierra. El menú contextual de la pestaña ofrece cerrar otras / cerrar a la derecha / duplicar. Deseable arrastrar una pestaña fuera de la ventana para convertirla en ventana propia.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+T` | Nueva pestaña |
| `Ctrl+W` | Cerrar la pestaña actual |
| `Ctrl+Tab / Ctrl+Shift+Tab` | Siguiente / anterior pestaña |
| `Ctrl+1 … Ctrl+8` | Ir directamente a la pestaña n (1-8) |
| `Ctrl+9` | Ir a la última pestaña (convención estilo navegador; no garantizada en todos los exploradores) |
| `Ctrl+PageDown / Ctrl+PageUp` | Nautilus / Nemo / Dolphin — siguiente / anterior pestaña |

> ℹ️ Windows 11 añadió pestañas en 2022 (Ctrl+T/Ctrl+W/Ctrl+número). Finder usa Cmd+T. Dolphin/Nautilus/Nemo admiten además Ctrl+PageUp/PageDown. Ctrl+9='última pestaña' es convención de Chrome/Firefox adoptada como diseño; no todos los exploradores la implementan de serie. Definir si la pestaña nueva hereda la carpeta actual o abre Inicio.

## Reabrir pestaña cerrada

**Categoría:** Pestañas y paneles  
**Prioridad:** ⚪ Opcional  
**Visto en:** KDE Dolphin

Recuperar la última pestaña cerrada, restaurando su ubicación (e idealmente su historial).

**Comportamiento esperado:** Ctrl+Shift+T reabre la pestaña cerrada más reciente en su carpeta; repetir el atajo reabre pestañas anteriores en orden inverso al de cierre. Es deseable restaurar también el historial atrás/adelante de esa pestaña y su posición en la barra. Debe llevar una pila acotada de pestañas cerradas (p.ej. las últimas 10).

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+Shift+T` | Reabrir la última pestaña cerrada (convención de navegador) |

> ℹ️ CORREGIDO: se retiran Nautilus y Nemo del seenIn por no ofrecer de forma fiable 'reabrir pestaña cerrada'; el caso confirmado es KDE Dolphin (menú 'Pestañas cerradas recientemente' / deshacer cierre). Muy esperado por costumbre de los navegadores (Ctrl+Shift+T). Windows 11 File Explorer no lo ofrece de forma nativa fiable, así que incluirlo es una ventaja competitiva.

## Abrir carpeta en pestaña o ventana nueva

**Categoría:** Pestañas y paneles  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus

Abrir una carpeta (desde la lista, el árbol o los accesos) en una pestaña o ventana nueva sin abandonar la actual.

**Comportamiento esperado:** El clic central sobre una carpeta la abre en una pestaña nueva (en segundo plano por defecto, configurable). El menú contextual ofrece 'Abrir en pestaña nueva' y 'Abrir en ventana nueva'. Ctrl+clic también abre en pestaña nueva. Ctrl+N abre una ventana nueva (de la ubicación actual o de Inicio, configurable). Se aplica igualmente a entradas del árbol de navegación y del Acceso rápido/Lugares. Al abrir en segundo plano, el foco permanece en la pestaña actual.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+N` | Nueva ventana |
| `Clic central del ratón` | Abrir la carpeta bajo el puntero en una pestaña nueva |
| `Ctrl+clic` | Abrir la carpeta en una pestaña nueva |

> ℹ️ El clic central para 'pestaña nueva' es un patrón heredado de los navegadores y muy valorado. Decidir (y hacer configurable) si la pestaña nueva se abre en primer o segundo plano. En Finder la nueva ventana es Cmd+N.

## Paneles dual / vista dividida

**Categoría:** Pestañas y paneles  
**Prioridad:** 🟡 Recomendable  
**Visto en:** KDE Dolphin, Cinnamon Nemo, Total Commander, Directory Opus

Mostrar dos carpetas lado a lado en la misma ventana para comparar y mover/copiar archivos entre ellas cómodamente.

**Comportamiento esperado:** Un atajo o botón divide la vista en dos paneles independientes, cada uno con su ruta, historial y selección propios. Tab (o F6) mueve el foco entre paneles; el panel activo se resalta claramente (borde/barra de título). Arrastrar, o F5/F6 al estilo comandante, transfiere archivos del panel activo al otro (el otro panel es el destino por defecto). Se puede intercambiar el contenido de los paneles y cerrar la división para volver a un solo panel. Cada panel puede tener sus propias pestañas. Deseable opción de división horizontal además de vertical.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `F3` | KDE Dolphin / Cinnamon Nemo — activar/desactivar la vista dividida (panel extra) |
| `Tab` | Cambiar el foco entre el panel izquierdo y el derecho |
| `F5 / F6` | Total Commander / Directory Opus — copiar / mover del panel activo al otro |

> ℹ️ Windows 11 File Explorer no ofrece paneles duales de serie (se suple con Snap/Ajustar ventanas). Es un rasgo estrella de los 'comandantes' (Total Commander, Directory Opus) y de Dolphin/Nemo (F3 'panel extra'). Muy pedido para gestión de archivos intensiva; encaja bien como diferenciador del proyecto.

## Doble clic en zona vacía para subir

**Categoría:** Navegación jerárquica  
**Prioridad:** ⚪ Opcional  
**Visto en:** Total Commander, Directory Opus, KDE Dolphin

Hacer doble clic sobre un área vacía del listado de archivos sube a la carpeta padre.

**Comportamiento esperado:** Un doble clic en el espacio en blanco del panel de archivos (donde no hay ningún elemento) navega a la carpeta contenedora, como atajo de ratón para 'Subir'. Debe ser una opción configurable y no interferir con la selección por recuadro (rubber-band) ni con el doble clic sobre elementos. Idealmente deja resaltada la carpeta de la que se venía. Desactivado por defecto por el riesgo de subidas accidentales.

> ℹ️ No existe en Windows 11 / Finder / Nautilus de serie; en Dolphin (opción 'Doble clic para subir') y en los comandantes es una opción. Al ser un proyecto de estética Windows 11, ofrecerlo desactivado por defecto y activable en Ajustes.

## Carpetas de resorte al arrastrar

**Categoría:** Navegación jerárquica  
**Prioridad:** ⚪ Opcional  
**Visto en:** macOS Finder, Windows 11 File Explorer, KDE Dolphin

Al arrastrar archivos, mantener el puntero sobre una carpeta (o un nodo del árbol / segmento de migas) la abre automáticamente para seguir navegando dentro y soltar en profundidad.

**Comportamiento esperado:** Durante una operación de arrastre, posar el cursor sobre una carpeta cerrada durante un breve instante (retardo configurable) la 'abre' temporalmente (spring-loaded), permitiendo profundizar varios niveles sin soltar. Si el puntero sale sin soltar, vuelve a la vista anterior. Se aplica también a los nodos del árbol y a los segmentos de la barra de direcciones (auto-expandir/navegar al mantener el arrastre encima). Al soltar, se realiza la operación (mover/copiar según modificador) en la carpeta destino final.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Barra espaciadora` | macOS Finder — abrir de inmediato la carpeta bajo el puntero durante el arrastre (sin esperar el retardo) |

> ℹ️ Finder es el referente (con retardo configurable y barra espaciadora para abrir ya). Windows auto-expande el árbol y las migas al mantener el arrastre encima. Ajustar el retardo para no abrir carpetas por accidente.

## Refrescar / recargar la vista

**Categoría:** Historial de navegación  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus)

Volver a leer el contenido de la carpeta actual para reflejar cambios hechos fuera del explorador (nuevos archivos, borrados, montajes).

**Comportamiento esperado:** Un atajo o botón vuelve a leer la carpeta actual y actualiza el listado, conservando en lo posible la selección y la posición de scroll. Aunque la vista debería refrescarse automáticamente mediante vigilancia del sistema de archivos (inotify en Linux), el refresco manual cubre casos donde la vigilancia falla (recursos de red, volúmenes montados, sistemas de archivos sin notificaciones). Debe reflejar también cambios de ordenación/agrupación pendientes. No debe reiniciar el historial ni cambiar de carpeta.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `F5` | Windows / Dolphin / Nemo / Nautilus — refrescar la vista (OJO: en comandantes ortodoxos F5 = COPIAR, no refrescar) |
| `Ctrl+R` | GNOME Files (Nautilus) / convención de navegador — recargar la vista |

> ℹ️ AÑADIDO (faltaba). Conflicto de grounding importante: F5 significa 'refrescar' en exploradores estándar pero 'copiar' en los comandantes ortodoxos (Total Commander, Directory Opus, Midnight Commander). Si el proyecto adopta gestos de comandante en el panel dual, resolver el conflicto (p.ej. F5=refrescar en modo normal, Ctrl+R como alternativa). macOS Finder no tiene refresco manual estándar (se apoya en actualización automática).

## Navegación con teclado en la lista de archivos

**Categoría:** Navegación jerárquica  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder

Moverse por el contenido de la carpeta y entrar/salir de carpetas usando solo el teclado, incluida la búsqueda rápida por teclas (type-ahead).

**Comportamiento esperado:** Las flechas mueven la selección (arriba/abajo en lista; también izquierda/derecha en vistas de iconos/cuadrícula). Enter abre/entra en la carpeta seleccionada (navegación descendente) o abre el archivo. Inicio/Fin saltan al primer/último elemento; Re Pág/Av Pág desplazan por páginas. Al escribir una o más letras, la selección salta al primer elemento cuyo nombre empieza por lo tecleado (type-ahead), con un pequeño tiempo de espera para acumular caracteres. Retroceso/Alt+Up sube al padre (según la convención elegida). La navegación con teclado debe mantener el elemento seleccionado siempre visible (auto-scroll).

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Flechas` | Mover la selección dentro de la carpeta |
| `Enter` | Entrar en la carpeta seleccionada / abrir el archivo |
| `Inicio / Fin` | Primer / último elemento de la carpeta |
| `Escribir letras` | Type-ahead: saltar al elemento que empieza por lo tecleado |
| `Retroceso` | Subir un nivel (según convención; en Windows equivale a 'Atrás') |

> ℹ️ AÑADIDO (faltaba una pieza básica de navegación). El type-ahead (saltar escribiendo el nombre) es universal en Windows/Finder y en Nautilus/Dolphin (que además pueden abrir una mini-búsqueda incremental al teclear). Distinguir type-ahead (selección por prefijo) de la búsqueda recursiva de la carpeta. Coherencia con la decisión de Retroceso='Atrás' vs 'Subir'.

## Copiar la ruta actual

**Categoría:** Barra de direcciones  
**Prioridad:** ⚪ Opcional  
**Visto en:** Windows 11 File Explorer, macOS Finder, KDE Dolphin, GNOME Files (Nautilus)

Copiar al portapapeles la ruta de la carpeta actual (o del elemento seleccionado) como texto, para pegarla en una terminal, un diálogo o un mensaje.

**Comportamiento esperado:** Una acción copia la ruta completa como texto plano. Desde la barra de direcciones: menú contextual 'Copiar dirección como texto'. Sobre un elemento seleccionado: 'Copiar como ruta de acceso'. Debe ofrecer la ruta en el formato nativo del sistema (POSIX en Linux) y, opcionalmente, entrecomillada si contiene espacios. Ctrl+L seguido de Ctrl+C también debe funcionar como camino alternativo. Con varios elementos seleccionados, copiar una ruta por línea.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+Shift+C` | Windows 11 — copiar la ruta del/los elemento(s) seleccionado(s) |
| `Cmd+Option+C` | macOS Finder — copiar la ruta como texto del elemento seleccionado |

> ℹ️ AÑADIDO (faltaba). Windows 11 expone 'Copiar como ruta de acceso' en el menú contextual y en la cinta (Ctrl+Shift+C). Finder usa Cmd+Option+C. En Linux, Dolphin/Nautilus permiten copiar la ubicación desde el campo de Ctrl+L o el menú contextual. Muy útil en un explorador orientado a usuarios que trabajan también con terminal.

## Restaurar pestañas y ubicación al iniciar

**Categoría:** Pestañas y paneles  
**Prioridad:** ⚪ Opcional  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, macOS Finder, Directory Opus

Al abrir el explorador, recuperar las pestañas y carpetas abiertas de la última sesión (o abrir una ubicación fija configurada).

**Comportamiento esperado:** En Ajustes se elige qué ocurre al abrir una ventana nueva o al iniciar la aplicación: restaurar las pestañas/ubicaciones de la sesión anterior, abrir siempre una ubicación fija (Inicio, carpeta personal) o abrir la última carpeta usada. Al restaurar, se recrean las pestañas con su ruta y, en lo posible, su historial y la vista dividida. Caso borde: si una ubicación guardada ya no existe (unidad desmontada), abrir su ancestro disponible o Inicio en su lugar, sin bloquear el arranque.

> ℹ️ AÑADIDO (faltaba). Windows tiene 'Restaurar las ventanas de carpetas anteriores al iniciar sesión' (Opciones de carpeta). Dolphin puede recordar las pestañas abiertas y el panel dividido. Finder reabre ventanas al reiniciar. Ofrecer la elección explícita: restaurar sesión vs. ubicación fija vs. última carpeta.

