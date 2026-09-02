# 3. Vistas, ordenación y organización

> Especificación de *conveniencias* (qué y cómo se comporta), agnóstica de implementación.

Conveniencias que controlan cómo se presenta, ordena y organiza el contenido de una carpeta: modos de vista, tamaño y densidad de los iconos, ordenación y agrupación, columnas configurables en Detalles, paneles (vista previa, detalles y barra lateral), miniaturas, opciones de visibilidad (archivos ocultos, extensiones), organización de iconos (posición libre/cuadrícula), plantillas de carpeta por tipo y persistencia de la configuración por carpeta. Todas están fundamentadas en al menos uno de: Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus y Total Commander. El objetivo es un explorador en Linux con estética y comodidades tipo Windows 11, adoptando el atajo estándar (normalmente el de Windows) y anotando variantes. Nota de alcance: las pestañas (Ctrl+T/Cmd+T) y la mayoría de la barra de rutas/breadcrumb se consideran parte del dominio de navegación y quedan fuera de esta especificación salvo el toggle de la barra lateral, incluido aquí como panel.

---

## Modos de vista (iconos extra grandes/grandes/medianos/pequeños, lista, detalles, mosaico, contenido)

**Categoría:** Modos de vista  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus

Distintas formas de presentar el contenido de una carpeta: rejillas de iconos de varios tamaños, lista compacta, tabla de detalles con columnas, mosaico con metadatos y vista de contenido.

**Comportamiento esperado:** Cambiar de modo re-dibuja la carpeta al instante conservando la selección y, en lo posible, dejando el elemento con foco visible tras el cambio. Iconos grandes/medianos muestran miniatura + nombre en una rejilla fluida que se re-empaqueta al redimensionar la ventana. 'Lista' apila nombres en columnas verticales sin metadatos, con flujo de arriba-abajo y luego a la derecha. 'Detalles' muestra una fila por elemento con columnas ordenables. 'Mosaico' presenta icono mediano con nombre, tipo y tamaño al lado. 'Contenido' muestra icono con nombre y metadatos enriquecidos en filas altas. Debe existir un modo por defecto sensato (Detalles para carpetas con muchos archivos; iconos grandes para carpetas de imágenes) y el modo elegido debe persistir por carpeta (ver 'Recordar la vista por carpeta').

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+Shift+1 … Ctrl+Shift+8` | Windows 11: 1=iconos extra grandes, 2=grandes, 3=medianos, 4=pequeños, 5=lista, 6=detalles, 7=mosaico, 8=contenido |
| `Ctrl+1 / Ctrl+2 / Ctrl+3` | KDE Dolphin: iconos / compacto / detalles. Cinnamon Nemo: iconos / lista / compacto |
| `Ctrl+1 / Ctrl+2` | GNOME Files (Nautilus): rejilla / lista |
| `Cmd+1 / Cmd+2 / Cmd+3 / Cmd+4` | macOS Finder: iconos / lista / columnas / galería |

> ℹ️ El número y nombre de modos varía: Nautilus moderno solo ofrece rejilla y lista; Windows es el más rico. Para estética tipo Windows conviene ofrecer al menos iconos (varios tamaños), lista, detalles y mosaico/contenido. La vista 'columnas' de Finder (navegación jerárquica por paneles) no tiene equivalente directo en Windows.

## Zoom de tamaño de icono con Ctrl+rueda

**Categoría:** Tamaño y densidad  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus

Aumentar o reducir el tamaño de los iconos y miniaturas manteniendo Ctrl y girando la rueda del ratón, o desde teclado.

**Comportamiento esperado:** En vista de iconos, cada muesca de rueda cambia el tamaño de forma incremental (idealmente fluida, no solo saltos entre modos). En Windows, en los extremos del recorrido también se transiciona entre modos de vista (p. ej. de iconos pequeños a Lista/Detalles/Contenido). El zoom se centra en el elemento bajo el cursor cuando es posible y conserva la selección. Debe haber límites mínimo/máximo razonables y un tamaño 'normal' al que volver. En vistas Detalles/Lista el gesto Ctrl+rueda suele cambiar de modo de vista en lugar de escalar.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+Rueda del ratón` | Sobre el área de archivos en Windows 11, KDE Dolphin, GNOME Files, Cinnamon Nemo y Directory Opus. En macOS Finder la rueda/scroll NO escala iconos: se usa el deslizador de Opciones de visualización (Cmd+J) o pellizcar en el trackpad |
| `Ctrl++ / Ctrl+-` | Aumentar/reducir zoom por teclado en KDE Dolphin, GNOME Files y Cinnamon Nemo |
| `Ctrl+0` | Restablecer al tamaño normal en GNOME Files (Nautilus) y Cinnamon Nemo |

> ℹ️ Corrección de grounding: Finder no tiene Cmd++/Cmd+- para el tamaño de icono ni escala con Ctrl+rueda (ese Ctrl+rueda en macOS es el zoom de accesibilidad de pantalla); su tamaño de icono se ajusta con el deslizador de Cmd+J o con pellizco en trackpad. La transición entre modos al llegar a los extremos es propia de Windows.

## Ordenar por (nombre, fecha de modificación, tipo, tamaño…)

**Categoría:** Ordenación  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Total Commander

Reordenar los elementos según un atributo: nombre, fecha de modificación/creación, tipo, tamaño y otros metadatos disponibles.

**Comportamiento esperado:** La ordenación es estable, insensible a mayúsculas por defecto en el nombre y con orden natural/numérico (archivo2 antes que archivo10). Las carpetas se agrupan por defecto antes que los archivos (ver 'Carpetas primero'). Cambiar el criterio re-ordena al instante conservando la selección y dejando el elemento con foco visible. Los criterios disponibles dependen de los metadatos (fecha, tamaño, tipo, etiquetas; dimensiones para imágenes; duración/álbum para vídeo/música). En vista Detalles, el criterio activo se refleja con un indicador (flecha) en la cabecera de columna. Debe existir un criterio de desempate estable (p. ej. nombre) cuando dos elementos comparten el valor del criterio principal.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+F3 / Ctrl+F4 / Ctrl+F5 / Ctrl+F6` | Total Commander: ordenar por nombre / extensión / fecha / tamaño (Ctrl+F7 = sin ordenar) |

> ℹ️ Windows y Finder no traen atajo global de teclado; se accede por menú contextual, menú Ver/Ordenar o por cabeceras en Detalles. El orden natural numérico es lo esperado (Windows lo hace nativamente); en Linux depende del locale y de la implementación (glib g_utf8_collate vs strcoll), por lo que conviene fijar un criterio de collation consistente e independiente del idioma para evitar sorpresas.

## Sentido de ordenación ascendente/descendente

**Categoría:** Ordenación  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Total Commander

Invertir el orden entre ascendente y descendente para el criterio de ordenación activo.

**Comportamiento esperado:** Alternar el sentido reordena manteniendo el criterio y la selección. En vista Detalles, hacer clic de nuevo en la misma cabecera invierte el sentido y la flecha (▲/▼) lo indica. Descendente por fecha muestra lo más reciente arriba (uso muy habitual). El sentido debe recordarse junto al criterio, por carpeta, de forma independiente.

> ℹ️ Es independiente del criterio y debe persistir por carpeta. Sin atajo de teclado estándar; se opera desde la cabecera de columna o el menú de ordenación.

## Carpetas primero

**Categoría:** Ordenación  
**Prioridad:** 🟡 Recomendable  
**Visto en:** KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Windows 11 File Explorer

Mantener las carpetas agrupadas antes (o después) de los archivos, con independencia del criterio de ordenación.

**Comportamiento esperado:** Opción para listar siempre las carpetas antes que los archivos. Al ordenar por tamaño o fecha, las carpetas siguen agrupadas arriba y se ordenan entre sí por ese mismo criterio. Debe poder desactivarse para mezclar carpetas y archivos (útil al ordenar por fecha para ver lo más reciente sin importar el tipo). En descendente, muchos exploradores mantienen las carpetas arriba igualmente. El estado del toggle debe persistir (global y/o por carpeta).

> ℹ️ Windows 11 fuerza carpetas primero y no ofrece desactivarlo con facilidad; Finder tiene 'Mantener carpetas arriba' (desde macOS Mojave); los exploradores Linux lo hacen configurable (Dolphin, Nautilus 'Ordenar carpetas antes que archivos', Nemo). Recomendado exponer el toggle.

## Ordenar con clic en cabecera de columna (vista Detalles)

**Categoría:** Ordenación  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Total Commander

En la vista Detalles, hacer clic en la cabecera de una columna ordena por ese atributo; un segundo clic invierte el sentido.

**Comportamiento esperado:** El clic en una cabecera fija el criterio y ordena ascendente; el segundo clic invierte a descendente. Se muestra una flecha ▲/▼ en la columna activa. El ancho de la columna se ajusta arrastrando su separador. Debe funcionar con cualquier columna visible (nombre, fecha, tipo, tamaño y columnas añadidas por el usuario). Cambiar de columna reinicia a ascendente por defecto.

> ℹ️ Es la forma más directa de ordenar en Detalles/Lista y la esperada por usuarios de Windows.

## Agrupar por atributo

**Categoría:** Agrupación  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, macOS Finder, KDE Dolphin, Directory Opus

Dividir la vista en grupos con cabecera según un atributo (tipo, fecha, tamaño, letra inicial del nombre).

**Comportamiento esperado:** Al agrupar aparecen cabeceras de sección (p. ej. 'Hoy', 'La semana pasada'; 'A–H'; 'Documentos'/'Imágenes'; rangos de tamaño). Los grupos deben poder colapsarse/expandirse y mostrar el recuento de elementos. La ordenación se aplica dentro de cada grupo. 'Sin agrupar' devuelve la lista plana. El agrupado combina con cualquier modo de vista y el criterio de agrupación puede diferir del de ordenación.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+Cmd+0` | macOS Finder: activar/desactivar 'Usar grupos' |

> ℹ️ Corrección de grounding: KDE Dolphin SÍ ofrece agrupación mediante 'Mostrar en grupos' (Show in Groups), que dibuja cabeceras de sección según el criterio de ordenación activo; añadido a seenIn. Cinnamon Nemo y GNOME Files (Nautilus) NO ofrecen 'Agrupar por' clásico. Windows lo trae completo (agrupar + ordenar dentro del grupo); es una comodidad muy visible en la estética Windows.

## Columnas configurables en Detalles

**Categoría:** Columnas  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Total Commander

Añadir, quitar y reordenar las columnas de metadatos mostradas en la vista Detalles.

**Comportamiento esperado:** El clic derecho en la barra de cabeceras muestra las columnas frecuentes con casillas para activarlas/desactivarlas y una opción 'Más…' con el catálogo completo (fecha de creación, autor, etiquetas, dimensiones, duración, bitrate, etc.). Las columnas se reordenan arrastrando la cabecera y se redimensionan arrastrando el separador. El conjunto y el ancho de columnas se recuerdan por carpeta (o por tipo/plantilla de carpeta). Debe ofrecer columnas contextuales según el contenido (Imágenes muestra Dimensiones; Música muestra Álbum/Duración). La columna de nombre no debe poder eliminarse.

> ℹ️ El catálogo de metadatos disponibles depende del backend de indexación (en Windows, propiedades del Shell; en Linux, Baloo/Tracker o lectura directa del archivo). En Finder las columnas de la vista Lista se eligen en Opciones de visualización (Cmd+J).

## Autoajustar ancho de columnas

**Categoría:** Columnas  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus

Ajustar el ancho de una columna (o de todas) al contenido más largo para que no se recorte el texto.

**Comportamiento esperado:** El doble clic en el separador entre cabeceras ajusta esa columna al contenido. Un comando 'Ajustar todas las columnas' las redimensiona todas a la vez. Debe respetar un ancho mínimo legible y no exceder el ancho de la ventana (recorta con puntos suspensivos si es necesario). Idealmente permite además fijar un ancho manual que se recuerde por carpeta.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl++ (teclado numérico)` | Windows: 'Cambiar el tamaño de todas las columnas para que se ajusten' (Ctrl y el '+' del teclado numérico) |
| `Doble clic en el separador de cabecera` | Ajustar una sola columna al contenido (todos los exploradores con vista Detalles) |

> ℹ️ El Ctrl++ del teclado numérico es específico de Windows; conviene mapear también un comando de menú 'Ajustar columnas' accesible sin numpad.

## Filtro por columna en la cabecera (vista Detalles)

**Categoría:** Columnas  
**Prioridad:** ⚪ Opcional  
**Visto en:** Windows 11 File Explorer, Directory Opus

Menú desplegable en cada cabecera de columna para filtrar los elementos por rangos o valores de ese atributo.

**Comportamiento esperado:** Al pasar el ratón o hacer clic en la flecha de la cabecera aparece un desplegable con casillas (por tipo; por rangos de fecha 'Hoy/Esta semana/Este mes'; por rangos de tamaño 'Pequeño/Mediano/Grande'; por letra inicial). Marcar valores filtra la vista in situ y muestra una marca de filtro activo en la cabecera. Es acumulable con la ordenación y la agrupación, y debe poder limpiarse de un solo gesto.

> ℹ️ Exclusivo de Windows y power-tools. Los exploradores Linux suelen resolver esto con un cuadro de búsqueda/filtro escrito en lugar de desplegables por columna.

## Panel de vista previa

**Categoría:** Paneles  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, macOS Finder, KDE Dolphin, Directory Opus

Panel lateral que muestra una vista previa del archivo seleccionado (imagen, texto, PDF, vídeo, audio).

**Comportamiento esperado:** Al seleccionar un archivo, el panel renderiza su contenido sin abrir la aplicación: imágenes a tamaño ajustado, texto/markdown/código con scroll, PDF paginado, vídeo/audio reproducibles. Con varios elementos seleccionados muestra el primero o un resumen. Debe cargar de forma asíncrona sin bloquear la navegación y evitar previsualizar archivos enormes o de tipo no soportado (con un mensaje de reemplazo). Se activa/desactiva y su ancho es ajustable y persistente.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Alt+P` | Windows 11: mostrar/ocultar el panel de vista previa |
| `Shift+Cmd+P` | macOS Finder: mostrar/ocultar previsualización |

> ℹ️ En Dolphin la previsualización vive en el 'Panel de información' (F11). Nautilus y Nemo carecen de panel de vista previa dedicado; usan la barra espaciadora (GNOME Sushi en Nautilus) para una previsualización flotante tipo Quick Look.

## Panel de detalles / información

**Categoría:** Paneles  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, macOS Finder, Directory Opus

Panel lateral con los metadatos del elemento seleccionado (tipo, tamaño, fecha, dimensiones, autor, etiquetas…).

**Comportamiento esperado:** Muestra las propiedades del elemento activo y, con selección múltiple, agregados (número de elementos y tamaño total). Algunas propiedades (etiquetas, valoración, autor) deben ser editables in situ cuando el sistema de archivos y el backend de metadatos lo permitan. Se activa/desactiva y su anchura persiste. No debe confundirse con la ventana modal de Propiedades.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Alt+Shift+P` | Windows 11: mostrar/ocultar el panel de detalles |
| `F11` | KDE Dolphin: mostrar/ocultar el panel de información |

> ℹ️ En Finder 'Obtener información' (Cmd+I) es una ventana aparte; el equivalente en panel es la previsualización de las vistas de columnas/galería. En Windows 11 el panel de detalles y el de vista previa son mutuamente excluyentes (ocupan el mismo lado).

## Mostrar/ocultar barra lateral (panel de navegación)

**Categoría:** Paneles  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, GNOME Files (Nautilus), Cinnamon Nemo, macOS Finder, Directory Opus

Alternar la visibilidad de la barra lateral de navegación (Accesos rápidos/Marcadores/Lugares/árbol) para ganar espacio horizontal.

**Comportamiento esperado:** El toggle oculta o muestra la barra lateral al instante; el área de archivos se re-empaqueta ocupando el espacio liberado. El estado persiste entre sesiones y la anchura de la barra es ajustable y se recuerda. Puede coexistir con los paneles de vista previa/detalles al otro lado de la ventana.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `F9` | KDE Dolphin (panel Lugares), GNOME Files (Nautilus) y Cinnamon Nemo: mostrar/ocultar la barra lateral |
| `Alt+Cmd+S` | macOS Finder: mostrar/ocultar la barra lateral |

> ℹ️ Incluido aquí por ser un panel/organización de la vista, aunque su contenido (marcadores, árbol) puede pertenecer también a un dominio de navegación. Windows 11 no trae atajo de teclado por defecto (se activa en Ver ▸ Mostrar ▸ Panel de navegación).

## Miniaturas de imágenes, vídeos y documentos

**Categoría:** Miniaturas  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Total Commander

Mostrar una miniatura del contenido real en lugar de un icono genérico para imágenes, vídeos, PDFs y otros formatos.

**Comportamiento esperado:** Genera las miniaturas en segundo plano y las cachea para no recalcularlas. Los vídeos muestran un fotograma representativo; los PDF/documentos, la primera página; las imágenes, una versión reducida con la orientación EXIF correcta. Debe existir una opción 'mostrar siempre iconos, nunca miniaturas' y límites configurables (no miniaturizar por encima de cierto tamaño de archivo, o hacerlo bajo demanda en unidades de red/extraíbles). Las miniaturas escalan con el zoom del icono y se refrescan si el archivo cambia (mtime).

> ℹ️ En Linux conviene usar el estándar de miniaturas de freedesktop (~/.cache/thumbnails, con la 'thumbnail managing standard') para compartir caché con otras apps. Ofrecer límite de tamaño y exclusión para unidades de red/extraíbles evita bloqueos.

## Mostrar/ocultar archivos ocultos

**Categoría:** Visibilidad  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Total Commander

Alternar la visibilidad de archivos y carpetas ocultos (nombres que empiezan por punto en Linux; atributo 'oculto' en Windows).

**Comportamiento esperado:** El toggle muestra/oculta al instante los elementos ocultos, que suelen dibujarse atenuados. Debe recordar la preferencia globalmente (y opcionalmente por carpeta). En Linux debe respetar también los ficheros listados en un '.hidden' de la carpeta. Conviene una segunda opción separada para 'archivos de sistema protegidos', distinta de los ocultos normales (como en Windows).

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+H` | GNOME Files (Nautilus), Cinnamon Nemo y KDE Dolphin: alternar ocultos (atajo estándar recomendado en Linux) |
| `Alt+.` | KDE Dolphin: atajo alternativo por defecto para ocultos |
| `Cmd+Shift+.` | macOS Finder: mostrar/ocultar archivos ocultos |

> ℹ️ Windows 11 no trae atajo de teclado por defecto (se activa en Ver ▸ Mostrar ▸ Elementos ocultos). El respeto de '.hidden' es propio de las apps GNOME (Nautilus, Nemo); Dolphin puede no soportarlo. Para un explorador en Linux con estética Windows, se recomienda adoptar Ctrl+H como estándar y aceptar Alt+. como alias.

## Mostrar extensiones de nombre de archivo

**Categoría:** Visibilidad  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, macOS Finder

Alternar la visualización de las extensiones (.txt, .jpg) en los nombres de archivo.

**Comportamiento esperado:** Con las extensiones ocultas, al renombrar solo se selecciona/edita el nombre base y la extensión se conserva. Mostrarlas ayuda a distinguir tipos y evita cambios accidentales de la extensión. Debe ser un ajuste global. Por seguridad conviene mostrar siempre la extensión real de ejecutables/scripts aunque el resto estén ocultas, para no enmascarar dobles extensiones engañosas (p. ej. 'factura.pdf.exe').

> ℹ️ En la mayoría de exploradores Linux (Dolphin/Nautilus/Nemo) las extensiones siempre se muestran, por lo que el toggle es propio de Windows/Finder. Para la estética tipo Windows conviene ofrecer el ajuste, con la extensión visible como valor por defecto seguro.

## Organización de iconos: posición libre, ajustar a la cuadrícula y organizar

**Categoría:** Organización de la vista  
**Prioridad:** ⚪ Opcional  
**Visto en:** macOS Finder, Windows 11 File Explorer, Directory Opus

En vista de iconos, controlar la disposición: colocación libre de iconos, alineado/ajuste a la cuadrícula, autoorganizar y 'organizar por' un atributo.

**Comportamiento esperado:** Ofrece 'Organización automática' (los iconos se recolocan solos y no dejan huecos), 'Alinear a la cuadrícula' (cada icono se ajusta a la celda más próxima al soltarlo) y un comando 'Organizar/Limpiar' que reordena todo a la cuadrícula. 'Organizar por' (Finder) ordena y bloquea la disposición según un atributo. Con posición libre desactivada la cuadrícula se re-empaqueta al redimensionar. La posición manual de los iconos, cuando se permite, debe recordarse por carpeta.

> ℹ️ Finder es el caso fuerte (Ver ▸ Limpiar / Ajustar a la cuadrícula / Organizar por). Windows 11 en ventanas de carpeta prácticamente auto-organiza y reserva la colocación libre para el Escritorio, así que para la estética Windows 11 esta comodidad es de baja prioridad; conviene al menos 'alinear a la cuadrícula'.

## Carpetas expandibles en vista Detalles/Lista

**Categoría:** Modos de vista  
**Prioridad:** ⚪ Opcional  
**Visto en:** macOS Finder, KDE Dolphin

Desplegar el contenido de las subcarpetas en línea, con triángulos de expansión, sin salir de la carpeta actual.

**Comportamiento esperado:** En Detalles/Lista, cada carpeta muestra un triángulo/flecha de despliegue; al expandirla, su contenido aparece indentado bajo ella y la ordenación se aplica dentro de cada nivel. Debe poder expandirse/colapsarse un nivel o todos los descendientes, y la selección puede abarcar varios niveles.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Cmd+Flecha derecha / Cmd+Flecha izquierda` | macOS Finder (vista Lista): expandir / colapsar la carpeta enfocada (Opt+Cmd+Flecha derecha expande todos los subniveles) |

> ℹ️ Finder (vista Lista) lo trae de serie; KDE Dolphin ofrece la opción 'Carpetas expandibles' en la vista Detalles. Windows 11 File Explorer NO expande carpetas en línea dentro de Detalles (usa el árbol del panel de navegación). Comodidad opcional para la estética Windows.

## Recordar la vista por carpeta

**Categoría:** Persistencia  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, macOS Finder, KDE Dolphin, Directory Opus

Memorizar de forma independiente el modo de vista, el criterio/sentido de ordenación, las columnas, el zoom y la agrupación de cada carpeta.

**Comportamiento esperado:** Al volver a una carpeta se restaura exactamente la configuración con que se dejó (por ejemplo, Detalles ordenada por fecha descendente con columnas personalizadas). Debe existir un modo global por defecto para las carpetas sin ajuste previo. Conviene limitar o purgar el histórico para que no crezca sin fin (Windows guardaba un número acotado de carpetas y podía 'olvidar' las más antiguas).

> ℹ️ Es la contraparte de 'Aplicar a todas las carpetas'. Ofrecer ambas y una acción de 'restablecer' evita que la memoria por carpeta se vuelva inconsistente.

## Aplicar la vista a todas las carpetas del mismo tipo

**Categoría:** Persistencia  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, macOS Finder, Directory Opus

Propagar la configuración de vista actual a todas las carpetas (o a todas las del mismo tipo/plantilla) de una sola vez.

**Comportamiento esperado:** Un comando 'Aplicar a todas las carpetas' fija la vista actual como predeterminada para el resto; 'Restablecer carpetas' vuelve al valor de fábrica. En Windows se aplica por plantilla de carpeta (General, Documentos, Imágenes, Música, Vídeos). Debe advertir de que sobrescribe las personalizaciones por carpeta existentes.

> ℹ️ En Windows está en Opciones de carpeta ▸ Ver; en Finder es 'Usar como predeterminado' dentro de Opciones de visualización (Cmd+J). La granularidad por 'tipo de carpeta' es una comodidad distintiva de Windows (ver 'Optimizar carpeta por tipo').

## Optimizar carpeta por tipo de contenido (plantillas de carpeta)

**Categoría:** Persistencia  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer

Asignar a una carpeta una plantilla según su contenido (General, Documentos, Imágenes, Música, Vídeos) que determina el modo de vista y las columnas por defecto.

**Comportamiento esperado:** Elegir 'Optimizar esta carpeta para…' cambia la vista y el conjunto de columnas predeterminados coherentes con ese tipo (p. ej. Imágenes → iconos grandes con miniaturas y columna Dimensiones; Música → Detalles con Álbum/Duración; Documentos → Detalles con Fecha de modificación). Puede aplicarse solo a la carpeta o también a sus subcarpetas. La detección puede ser automática al abrir una carpeta sin ajuste previo.

> ℹ️ En Windows está en Propiedades de la carpeta ▸ Personalizar ▸ 'Optimizar esta carpeta para'. GNOME/KDE no tienen un equivalente directo; es una comodidad distintiva de Windows que complementa 'Aplicar la vista a todas las carpetas del mismo tipo' y encaja bien con la estética Windows.

## Densidad / espaciado de la vista (modo compacto)

**Categoría:** Tamaño y densidad  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, Directory Opus

Ajustar el espaciado entre elementos para mostrar más contenido (modo compacto) o una disposición más aireada, sin cambiar el tamaño del icono.

**Comportamiento esperado:** Un toggle 'Vista compacta' reduce el alto de fila y el margen entre iconos para ver más elementos manteniendo el mismo tamaño de icono. Debe respetar objetivos de accesibilidad (áreas de clic mínimas). Es combinable con cualquier modo de vista y distinto del zoom de icono: aquí cambia la separación, no el tamaño del icono. El estado persiste globalmente.

> ℹ️ Corrección de grounding: reducido seenIn a Windows 11 (que introdujo 'Vista compacta' precisamente porque su espaciado por defecto es amplio) y Directory Opus (que permite configurar padding/espaciado de fila). KDE Dolphin, GNOME Files y Cinnamon Nemo NO tienen un toggle de densidad independiente del modo de vista. Para una estética fiel conviene replicar tanto el espaciado amplio por defecto como el toggle compacto.

## Barra de estado con recuento, selección y tamaño

**Categoría:** Organización de la vista  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Total Commander

Barra inferior que informa del número de elementos de la carpeta, cuántos hay seleccionados y el tamaño total de la selección.

**Comportamiento esperado:** Muestra 'N elementos'; al seleccionar, 'X de N seleccionados' y el tamaño sumado, actualizándose en tiempo real. Con un único archivo seleccionado muestra su tamaño; con carpetas, opcionalmente su tamaño calculado. Puede activarse/desactivarse.

> ℹ️ Corrección de grounding: los botones rápidos de cambio de vista y el deslizador de zoom en la esquina inferior derecha eran de Windows 10; Windows 11 los retiró de la barra de estado, que ahora solo muestra recuento/selección. Para una estética a lo Windows 10 conviene replicar esos controles de esquina; para Windows 11 basta con recuento y selección.

## Vista de doble panel / dividida

**Categoría:** Organización de la vista  
**Prioridad:** ⚪ Opcional  
**Visto en:** KDE Dolphin, Cinnamon Nemo, Total Commander, Directory Opus

Mostrar dos listados de carpeta lado a lado en la misma ventana para comparar y mover/copiar entre ambos.

**Comportamiento esperado:** Dos paneles independientes, cada uno con su propia ruta, modo de vista y ordenación; arrastrar entre paneles mueve o copia. Un panel está 'activo' (con foco) y Tab alterna entre ellos. Debe poder cerrarse para volver a panel único y recordar el estado. Muy valorado para la gestión intensiva de archivos.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `F3` | KDE Dolphin y Cinnamon Nemo: dividir/unir la vista (panel adicional) |
| `Tab` | Total Commander: alternar el panel activo (es inherentemente de doble panel) |

> ℹ️ Ni Windows 11 File Explorer ni macOS Finder lo traen de serie (Windows 11 apostó por pestañas). Es una comodidad clásica de Linux y power-tools; opcional para el proyecto, pero muy apreciada por usuarios avanzados en Linux.

