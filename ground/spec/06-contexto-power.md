# 6. Menú contextual, acciones y power-user

> Especificación de *conveniencias* (qué y cómo se comporta), agnóstica de implementación.

Especificación exhaustiva de las conveniencias de cara al usuario del dominio de menú contextual, acciones sobre archivos y flujos de power-user en un explorador de archivos moderno con estética y comodidades tipo Windows 11 sobre Linux. Cubre el menú contextual (clic derecho y tecla Menú), portapapeles y operaciones de copiar/mover, deshacer/rehacer, eliminar/papelera, selección múltiple, arrastrar y soltar con modificadores, "Abrir con", "Enviar a", compresión y extracción, creación desde plantilla, renombrado (individual y por lotes), abrir terminal en la ubicación, copiar ruta, crear accesos directos/enlaces, vista rápida con Espacio, panel de vista previa/detalles acoplado, propiedades, marcadores/favoritos, fijar a Acceso rápido, pestañas, paneles duales, montar/expulsar unidades, acciones/scripts personalizados y ejecución elevada. Cada conveniencia está fundamentada en al menos un explorador real (Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files/Nautilus, macOS Finder y power-tools como Directory Opus y Total Commander) con atajos verificados y variantes documentadas. 26 conveniencias.

---

## Menú contextual (clic derecho y tecla Menú)

**Categoría:** Menú contextual  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder

Menú emergente con acciones relevantes al elemento o zona bajo el puntero, invocado con clic derecho o con la tecla Menú/Shift+F10 sobre la selección activa.

**Comportamiento esperado:** Debe adaptar sus opciones al contexto (archivo suelto, carpeta, selección múltiple, fondo vacío del panel, elemento de la barra lateral). En Windows 11 el menú es compacto con acciones frecuentes como iconos en cabecera (Cortar, Copiar, Renombrar, Compartir, Eliminar) y una entrada 'Mostrar más opciones' que despliega el menú clásico extendido. Debe abrirse junto al cursor sin salirse de la pantalla, reposicionándose si no cabe; cerrarse con Esc; navegarse con flechas y aceleradores por letra subrayada; y con selección múltiple mostrar solo acciones aplicables a todos los elementos (las no comunes se ocultan o deshabilitan). Clic derecho en zona vacía ofrece Nuevo, Pegar, Ordenar por, Ver, Agrupar y Actualizar. La tecla Menú y Shift+F10 lo abren centrado en la selección o junto a ella. Caso borde: en carpetas virtuales o remotas algunas acciones (abrir terminal, ejecutar) se deshabilitan u ocultan.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Menu` | Tecla de menú contextual sobre la selección |
| `Shift+F10` | Alternativa universal; en Windows 11 abre directamente el menú clásico/ampliado |
| `Esc` | Cerrar el menú |

> ℹ️ Windows 11 divide el menú en compacto más 'Mostrar más opciones' (o Shift+F10, que salta directo al menú clásico); Linux y macOS usan un único menú. Para la estética Windows 11 conviene replicar la fila de iconos de acciones rápidas en la cabecera y evitar que el menú compacto quede demasiado escueto.

## Cortar, copiar y pegar

**Categoría:** Portapapeles  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Total Commander

Operaciones de portapapeles sobre archivos y carpetas desde el menú contextual o el teclado: mover (cortar), duplicar (copiar) y pegar entre carpetas, pestañas y ventanas.

**Comportamiento esperado:** Cortar deja los elementos atenuados hasta que se pegan (se mueven) y se cancela con Esc o al iniciar otra copia; Copiar los duplica. Pegar en el destino resuelve conflictos de nombre con un diálogo Reemplazar / Omitir / Conservar ambos (renombrado automático con sufijo), comparando fecha y tamaño de origen y destino en cada conflicto y permitiendo aplicar la decisión a todos. Pegar en la misma carpeta genera una copia con sufijo ('- copia' / '(1)'); cortar y pegar en la misma carpeta no hace nada. Debe soportar pegar accesos directos, mostrar barra de progreso con velocidad y cancelación en operaciones largas, y respetar el orden actual al pegar en zona vacía.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+X` | Cortar (mover) |
| `Ctrl+C` | Copiar |
| `Ctrl+V` | Pegar |

> ℹ️ macOS Finder no tiene 'Cortar' clásico: se copia con Cmd+C y se mueve al pegar con Cmd+Option+V. En Windows y Linux el flujo cortar-pegar (Ctrl+X/Ctrl+V) es el estándar esperado.

## Deshacer y rehacer operaciones

**Categoría:** Acciones  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder

Deshacer y rehacer la última operación de archivos (mover, copiar, renombrar, crear, enviar a la papelera) sin depender del historial del sistema.

**Comportamiento esperado:** Ctrl+Z deshace la última operación reversible: revierte un renombrado, devuelve a su origen los elementos movidos, elimina la copia recién pegada, restaura desde la papelera lo enviado a ella o borra la carpeta/plantilla recién creada. Ctrl+Y (o Ctrl+Shift+Z) rehace. Debe mantener una pila de varias operaciones y, tras una acción reversible, mostrar un aviso con enlace 'Deshacer' (barra de notificación en Nautilus, notificación/toast en Windows). Casos borde: el borrado permanente (Mayús+Supr / vaciar papelera) no es reversible y debe advertirse; algunas operaciones sobre volúmenes remotos o de solo lectura no se pueden deshacer y la acción se deshabilita; deshacer un movimiento cuyo destino ya cambió pide confirmación.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+Z` | Deshacer (Windows 11, Nautilus, Dolphin, Nemo) |
| `Ctrl+Y` | Rehacer (Windows 11, Dolphin) |
| `Ctrl+Shift+Z` | Rehacer (Nautilus, Nemo) |
| `Cmd+Z` | Deshacer en macOS Finder (Cmd+Shift+Z rehace) |

> ℹ️ Nautilus y Nemo muestran una barra de notificación con 'Deshacer' tras mover/borrar. El deshacer de operaciones de archivo es distinto del deshacer de texto durante el renombrado.

## Eliminar (papelera) y borrado permanente

**Categoría:** Papelera  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder

Enviar a la papelera con Supr y borrar de forma permanente con Mayús+Supr, con restauración desde la papelera y confirmaciones configurables.

**Comportamiento esperado:** Supr mueve la selección a la papelera/Papelera de reciclaje (reversible, con 'Restaurar'/'Devolver' a la ubicación original). Mayús+Supr borra de forma permanente tras confirmación. En macOS, Cmd+Supr envía a la papelera, Cmd+Mayús+Supr la vacía y Opción+Cmd+Supr borra de inmediato. La papelera conserva la ubicación original para poder restaurar. Debe existir una opción para pedir o no confirmación al enviar a la papelera. Casos borde: elementos en unidades de red, extraíbles o remotas suelen omitir la papelera y se borran directamente (hay que avisar de ello); un archivo abierto/bloqueado o sin permisos falla con mensaje claro; borrar carpetas grandes muestra progreso y permite cancelar.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Delete` | Enviar a la papelera (Windows 11, Dolphin, Nemo, Nautilus) |
| `Shift+Delete` | Borrado permanente (Windows 11, Dolphin, Nautilus tras confirmar) |
| `Cmd+Delete` | Enviar a la papelera en macOS Finder |
| `Cmd+Shift+Delete` | Vaciar la papelera en macOS Finder |

> ℹ️ En Linux/GNOME/KDE, algunas configuraciones ocultan 'Eliminar permanentemente' por defecto y lo revelan con Mayús. El borrado permanente es irreversible y no debe ofrecerse como acción por defecto sin confirmación.

## Selección múltiple (clic, banda elástica, seleccionar todo)

**Categoría:** Selección  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Total Commander

Seleccionar varios elementos por clic con modificadores, banda elástica y atajos de seleccionar todo, ninguno e invertir, con conteo y tamaño en la barra de estado.

**Comportamiento esperado:** Clic selecciona uno; Ctrl+clic añade o quita elementos sueltos de la selección; Mayús+clic selecciona un rango contiguo desde el ancla; arrastrar sobre el fondo vacío dibuja una banda elástica de selección. Ctrl+A selecciona todo; el menú/Cinta ofrece 'No seleccionar nada' e 'Invertir selección'. La escritura rápida (type-ahead) salta al primer elemento que empieza por lo tecleado. Debe mostrar el número de elementos seleccionados y su tamaño total en la barra de estado. Casos borde: Ctrl+A mientras se edita un nombre selecciona el texto, no los archivos; la banda elástica respeta el modo de vista (icono/lista/columnas).

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+A` | Seleccionar todo (Windows 11, Dolphin, Nemo, Nautilus) |
| `Cmd+A` | Seleccionar todo en macOS Finder |
| `Ctrl+Click` | Alternar la selección de un elemento suelto |
| `Shift+Click` | Seleccionar un rango contiguo |

> ℹ️ 'Invertir selección' existe en el menú de Windows (y en Dolphin) pero no tiene un atajo universal por defecto. En macOS, Cmd+clic/Mayús+clic cumplen el papel de Ctrl+clic/Mayús+clic.

## Abrir con

**Categoría:** Aplicaciones  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder

Submenú para abrir el archivo con una aplicación distinta a la predeterminada y para elegir o cambiar la aplicación por defecto del tipo de archivo.

**Comportamiento esperado:** Lista las aplicaciones compatibles con el tipo (incluidas las usadas recientemente) y ofrece 'Elegir otra aplicación' para explorar todas las instaladas y, opcionalmente, fijar una como predeterminada mediante una casilla 'usar siempre'. La selección múltiple del mismo tipo abre todos los elementos con la app elegida; con tipos distintos se aplica por tipo o se limita la opción de app única. Los tipos sin asociación ofrecen buscar una aplicación, abrir con un editor genérico o buscar en la tienda/repositorio.

> ℹ️ Windows 11: 'Abrir con > Elegir otra aplicación'. En macOS Finder, mantener Option convierte 'Abrir con' en 'Abrir siempre con'. En Linux respeta las asociaciones MIME del sistema (.desktop / MimeType).

## Abrir en nueva pestaña o ventana

**Categoría:** Navegación  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder

Acciones para abrir una carpeta en una pestaña nueva, en una ventana nueva o (en vista de paneles) en el panel opuesto, sin abandonar la ubicación actual.

**Comportamiento esperado:** El clic central sobre una carpeta la abre en una pestaña nueva en segundo plano; el menú contextual ofrece 'Abrir en nueva pestaña' y 'Abrir en nueva ventana'. En vista de paneles duales añade 'Abrir en el otro panel'. Al seleccionar varias carpetas puede abrir cada una en su propia pestaña. La pestaña nueva hereda el modo de vista y el orden actuales. Ctrl+N abre una ventana nueva del explorador.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Middle-Click` | Abrir la carpeta en una pestaña nueva en segundo plano (Dolphin, Nemo, Nautilus, Windows 11) |
| `Ctrl+N` | Abrir una ventana nueva del explorador (Windows 11, Nautilus, Dolphin, Nemo) |

> ℹ️ Corrección de grounding: en los gestores de archivos, Ctrl+clic ALTERNA la selección y Mayús+clic selecciona un RANGO; no abren pestaña ni ventana (eso es comportamiento de navegadores web). El clic central para abrir en pestaña de fondo sí es muy consistente entre Dolphin, Nemo, Nautilus y Windows 11 y lo esperan los power-users. La nueva ventana se abre por menú contextual o con Ctrl+N.

## Enviar a

**Categoría:** Menú contextual  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, GNOME Files (Nautilus)

Submenú de destinos rápidos para la selección: crear acceso directo en el escritorio, carpeta comprimida, destinatario de correo, y dispositivos o unidades detectados.

**Comportamiento esperado:** Ofrece destinos predefinidos (Escritorio como acceso directo, Carpeta comprimida (zip), Destinatario de correo, Documentos, unidades extraíbles detectadas dinámicamente) y es extensible por el usuario añadiendo accesos a la carpeta 'SendTo'. 'Carpeta comprimida' crea un zip con la selección; las unidades aparecen al conectarlas y desaparecen al retirarlas.

> ℹ️ Es un concepto muy de Windows; Nautilus tiene un 'Enviar a...' limitado (correo). En Linux se replica combinando 'Comprimir', 'Crear enlace' y 'Copiar a'. Mantener el submenú 'Enviar a' refuerza la estética Windows 11 objetivo.

## Copiar a / Mover a (selector de carpeta)

**Categoría:** Portapapeles  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, Directory Opus

Acciones que copian o mueven la selección a una carpeta elegida mediante un selector o una lista de marcadores, sin pasar por el portapapeles.

**Comportamiento esperado:** Abre un árbol/diálogo para elegir el destino (con acceso a marcadores y ubicaciones recientes) y ejecuta la operación con barra de progreso, resolución de conflictos y opción de cancelar. En Dolphin y Nemo el submenú lista directamente los marcadores como destinos de un clic. Debe permitir crear una carpeta nueva desde el selector; mover a la misma carpeta no hace nada.

> ℹ️ En Windows aparece en el menú clásico ('Mover a'/'Copiar a'). En Dolphin y Nemo integra la lista de marcadores como destinos rápidos, algo muy cómodo para power-users.

## Arrastrar y soltar con modificadores

**Categoría:** Arrastrar y soltar  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Total Commander

Arrastrar archivos entre carpetas, pestañas, paneles y ventanas, con modificadores que fuerzan copiar, mover o crear un enlace/acceso directo.

**Comportamiento esperado:** Arrastrar sin modificador aplica la acción por defecto (mover dentro del mismo volumen, copiar entre volúmenes distintos). Ctrl fuerza copiar, Mayús fuerza mover y Ctrl+Mayús (o Alt en Windows) crea un acceso directo/enlace; el cursor muestra el signo +, la flecha de mover o el emblema de enlace. Arrastrar con el botón derecho (Windows) o el central (GNOME/KDE) abre al soltar un menú 'Copiar aquí / Mover aquí / Crear enlace aquí / Cancelar'. En macOS, arrastrar mueve dentro del volumen, copia entre volúmenes, Cmd fuerza mover, Opción fuerza copiar y Cmd+Opción crea un alias. Soltar sobre una carpeta, una pestaña o el otro panel usa ese destino; mantener el puntero sobre una carpeta o pestaña la abre (spring-loaded). Casos borde: soltar en una ubicación de solo lectura se rechaza con cursor de prohibido; soltar un elemento sobre sí mismo no hace nada.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+Drag` | Forzar copiar (Windows, Linux) |
| `Shift+Drag` | Forzar mover (Windows, Linux) |
| `Ctrl+Shift+Drag` | Crear enlace/acceso directo (Windows; Alt+Drag como alternativa en Windows) |
| `Option+Drag` | Forzar copiar en macOS Finder (Cmd+Option+Drag crea alias) |

> ℹ️ El arrastre con botón derecho (Windows) o central (Linux) que muestra un menú al soltar evita ambigüedad entre copiar/mover/enlazar y lo valoran los power-users.

## Comprimir y extraer archivos

**Categoría:** Archivado  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, GNOME Files (Nautilus), KDE Dolphin, Cinnamon Nemo, macOS Finder

Crear archivos comprimidos (ZIP, 7z, tar) desde la selección y extraer archivos comprimidos existentes desde el menú contextual.

**Comportamiento esperado:** 'Comprimir a' ofrece formatos (ZIP, 7z, tar.gz según soporte) y crea el archivo en la misma carpeta con el nombre editable. 'Extraer todo' descomprime a una subcarpeta con el nombre del archivo o a la ubicación elegida; 'Extraer aquí' vuelca el contenido en la carpeta actual. Debe mostrar progreso, permitir ver el contenido antes de extraer, y resolver conflictos preguntando reemplazar/omitir. Los archivos protegidos con contraseña solicitan la clave; una extracción parcial o fallida debe informarse.

> ℹ️ Windows 11 añadió compresión nativa a 7z y tar además de zip, y extracción de rar/7z/otros formatos vía libarchive. macOS Finder solo comprime a .zip y extrae con doble clic. En Linux depende del backend (Ark en Dolphin, File Roller en Nautilus, o el gestor de Nemo).

## Nuevo (carpeta y desde plantilla)

**Categoría:** Creación  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder

Submenú 'Nuevo' para crear una carpeta o documentos vacíos a partir de plantillas (documento de texto, hoja de cálculo, etc.) listadas desde una carpeta de plantillas del usuario.

**Comportamiento esperado:** 'Nueva carpeta' crea la carpeta y la deja seleccionada en modo de edición de nombre, lista para escribir. 'Nuevo > <plantilla>' copia la plantilla y la deja renombrable. En GNOME y Cinnamon las plantillas provienen de ~/Templates (o ~/Plantillas); en Windows del menú 'Nuevo' que registran las aplicaciones. Crear en zona vacía coloca el elemento junto al puntero; los nombres duplicados reciben un sufijo automático.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+Shift+N` | Nueva carpeta (Windows 11, Nautilus, Nemo) |
| `F10` | Crear carpeta en KDE Dolphin (atajo por defecto de 'Crear nuevo > Carpeta') |
| `Cmd+Shift+N` | Nueva carpeta en macOS Finder |

> ℹ️ La nueva carpeta debe quedar seleccionada y con el nombre en edición. Dolphin usa F10 por defecto para crear carpeta, lo que puede chocar con expectativas de otros exploradores; conviene aceptar también Ctrl+Shift+N.

## Renombrar (individual y por lotes)

**Categoría:** Creación  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Total Commander

Renombrar un elemento en línea con F2 y renombrar por lotes varios elementos con numeración o patrones de búsqueda/reemplazo.

**Comportamiento esperado:** F2 entra en la edición del nombre con la parte del nombre seleccionada (sin la extensión). Con varios elementos seleccionados, F2 aplica un renombrado por lotes: patrón con contador (por ejemplo 'Foto #1', 'Foto #2') o buscar y reemplazar sobre los nombres, con vista previa antes de aplicar. Enter confirma, Esc cancela, Tab pasa al siguiente en el renombrado inline múltiple. Los caracteres inválidos y nombres reservados se rechazan con aviso; cambiar solo la extensión pide confirmación.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `F2` | Renombrar (Windows 11, Dolphin, Nemo, Nautilus) |
| `Enter` | Renombrar en macOS Finder (Return) |

> ℹ️ Windows selecciona el nombre sin la extensión al renombrar. Dolphin y Nautilus ofrecen un potente renombrado por lotes con contador o buscar/reemplazar; Directory Opus y Total Commander incluyen renombrado avanzado con expresiones regulares.

## Abrir terminal aquí

**Categoría:** Terminal  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder

Abrir una terminal ya situada en la carpeta actual, o alternar un panel de terminal integrado dentro del explorador que sigue la navegación.

**Comportamiento esperado:** Desde el menú contextual (fondo o carpeta) lanza la terminal predeterminada con el directorio de trabajo en la ubicación. Dolphin integra además un panel de terminal empotrado que sigue la carpeta activa y se alterna con F4. En carpetas remotas o virtuales la acción se deshabilita o abre en la ruta montada. Debe respetar la terminal preferida del usuario.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `F4` | Alternar el panel de terminal integrado en KDE Dolphin |

> ℹ️ Windows 11 trae 'Abrir en Terminal' que abre Windows Terminal en la ruta. Nautilus lo requiere vía extensión (nautilus-open-terminal). macOS necesita activar 'Nuevo terminal en la carpeta' en Ajustes. La integración F4 de Dolphin es muy valorada por power-users.

## Copiar ruta (como texto)

**Categoría:** Rutas  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, macOS Finder

Copiar al portapapeles la ruta completa del elemento o de la carpeta actual como texto, para pegarla en la terminal, en diálogos o en documentos.

**Comportamiento esperado:** Copia la ruta absoluta; en Windows entre comillas cuando contiene espacios. Debe permitir copiar la ruta de la selección o de la carpeta actual. La selección múltiple copia una ruta por línea. Las rutas de red usan formato UNC en Windows; conviene ofrecer copiar en estilo POSIX o Windows según la plataforma de destino.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+Shift+C` | Copiar como ruta de acceso (Windows 11) |
| `Ctrl+Alt+C` | Copiar ubicación en KDE Dolphin (variante; la acción 'Copiar ubicación' tiene atajo configurable y puede no venir asignado por defecto) |
| `Cmd+Option+C` | Copiar como nombre de ruta en macOS Finder |

> ℹ️ Windows 11 lo trae en el menú compacto ('Copiar como ruta de acceso', Ctrl+Shift+C). macOS Finder usa Cmd+Option+C. En Nautilus y Nemo no hay 'copiar ruta' dedicado por defecto: Ctrl+C copia el elemento y al pegar en un campo de texto se obtiene la ruta (o se usa Ctrl+L para mostrar la barra de dirección editable). Es una acción muy usada por desarrolladores.

## Crear acceso directo / enlace

**Categoría:** Enlaces  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, GNOME Files (Nautilus), Cinnamon Nemo, KDE Dolphin, macOS Finder, Total Commander

Crear un acceso directo (Windows .lnk), un enlace simbólico (Linux) o un alias (macOS) que apunta al elemento original.

**Comportamiento esperado:** Genera el enlace en la misma carpeta (con sufijo '- acceso directo') o permite arrastrar con un modificador a otra ubicación para crearlo allí. En Linux distingue entre enlace simbólico y enlace duro. El enlace debe mostrar un emblema o flecha para distinguirse del original. Mover el original rompe un enlace simbólico, mientras que el alias de macOS lo vuelve a resolver.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+Shift+Drag` | Arrastrar creando un enlace/acceso directo (Windows 11, Nautilus) |
| `Ctrl+Cmd+A` | Crear alias en macOS Finder |

> ℹ️ La terminología difiere: acceso directo (Windows), enlace (Linux), alias (macOS). Mostrar el emblema de enlace ayuda a no confundirlos con el archivo real.

## Vista rápida con Espacio (Quick Look)

**Categoría:** Vista previa  
**Prioridad:** 🟡 Recomendable  
**Visto en:** macOS Finder, GNOME Files (Nautilus)

Previsualización instantánea del archivo seleccionado (imagen, PDF, texto, vídeo, audio) en una ventana flotante al pulsar la barra espaciadora, sin abrir la aplicación completa.

**Comportamiento esperado:** Al seleccionar un elemento y pulsar Espacio se muestra una previsualización grande flotante; con las flechas se recorre la selección manteniendo abierta la vista; Espacio o Esc la cierran. Debe soportar reproducción de multimedia y desplazamiento dentro de documentos. Los tipos sin previsualizador muestran el icono e información básica; los archivos grandes cargan de forma diferida.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Space` | Abrir/cerrar la vista rápida del elemento seleccionado |
| `Esc` | Cerrar la vista rápida |

> ℹ️ Es una comodidad clave que Windows no trae de fábrica (usa un panel de vista previa lateral con Alt+P; apps como QuickLook o Seer la replican). En Nautilus la aporta 'Sushi'. Replicarla con Espacio da mucho valor a un explorador estilo Windows 11 en Linux. No confundir con el panel de vista previa lateral acoplado (ver 'preview-details-pane').

## Panel de vista previa / detalles acoplado

**Categoría:** Vista previa  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, macOS Finder

Panel lateral acoplado que muestra una vista previa y/o los metadatos del elemento seleccionado, distinto de la vista rápida flotante con Espacio.

**Comportamiento esperado:** Un panel a la derecha (o lateral) muestra la miniatura/vista previa y metadatos (tamaño, dimensiones, fecha, etiquetas, permisos) del elemento seleccionado y se actualiza al cambiar la selección, sin abrir la app. En Windows, Alt+P alterna el panel de vista previa y Alt+Mayús+P el panel de detalles; Dolphin alterna el panel de Información con F11 (miniatura + metadatos + reproducción básica); Finder muestra el panel de vista previa con Mayús+Cmd+P y en vista de columnas incluye la previsualización. Debe permitir ajustar el ancho del panel y funcionar con multimedia (reproducción ligera). Casos borde: con selección múltiple resume el recuento y el tamaño total; sin selección muestra información de la carpeta.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Alt+P` | Alternar el panel de vista previa (Windows 11) |
| `Alt+Shift+P` | Alternar el panel de detalles (Windows 11) |
| `F11` | Alternar el panel de Información en KDE Dolphin |
| `Shift+Cmd+P` | Mostrar/ocultar el panel de vista previa en macOS Finder |

> ℹ️ No sustituye a la vista rápida a pantalla con Espacio: son conveniencias complementarias. Nautilus retiró su panel de detalles lateral y se apoya en Sushi (Espacio) y en las columnas de la vista de lista.

## Propiedades / Información

**Categoría:** Menú contextual  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder

Diálogo con los detalles del elemento: tamaño, fechas, tipo, ubicación, permisos y pestañas de seguridad/compartición o de aplicación predeterminada.

**Comportamiento esperado:** Alt+Enter abre las propiedades de la selección; con varios elementos suma los tamaños y cuenta archivos y carpetas. Permite cambiar atributos y permisos, la aplicación predeterminada, y ver el uso de disco calculado de forma incremental en segundo plano sin bloquear el diálogo. Con selección de tipos distintos se ocultan las pestañas no comunes.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Alt+Enter` | Propiedades (Windows 11, Dolphin, Nemo, Nautilus) |
| `Ctrl+I` | Propiedades en GNOME Files reciente (variante de Nautilus) |
| `Cmd+I` | Obtener información en macOS Finder |

> ℹ️ macOS lo llama 'Obtener información' (Cmd+I) y ofrece la ventana Inspector con Cmd+Option+I. Nautilus moderno acepta Ctrl+I además de Alt+Enter. El cálculo de tamaño de carpetas grandes debe ser incremental y no bloquear el diálogo.

## Marcadores / Favoritos (barra lateral)

**Categoría:** Favoritos  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder

Lista de ubicaciones favoritas en la barra lateral para saltar a carpetas de uso frecuente con un clic, con posibilidad de añadir, renombrar y reordenar.

**Comportamiento esperado:** Se añade la carpeta actual con Ctrl+D o arrastrándola a la barra lateral; se reordena por arrastre; el marcador puede renombrarse sin cambiar la carpeta real. La barra separa secciones (favoritos, dispositivos, red). Un marcador a una ubicación inexistente se marca como no disponible; soporta carpetas remotas.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+D` | Añadir la ubicación actual a los marcadores (Nautilus, Nemo) |
| `Ctrl+Cmd+T` | Añadir a la barra lateral en macOS Finder |

> ℹ️ En Windows el equivalente es 'Acceso rápido'/'Inicio'; en Finder son los 'Favoritos' de la barra lateral. Poder reordenar por arrastre es un comportamiento esperado.

## Fijar a Acceso rápido / Inicio

**Categoría:** Favoritos  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, macOS Finder, GNOME Files (Nautilus), Cinnamon Nemo, KDE Dolphin

Anclar carpetas al área de Acceso rápido (Inicio) de la barra lateral y de la página de inicio, junto al listado automático de carpetas frecuentes y archivos recientes.

**Comportamiento esperado:** 'Fijar a Acceso rápido' ancla la carpeta en la parte superior de la barra lateral; las no fijadas aparecen por frecuencia de uso y pueden quitarse con 'Quitar de Acceso rápido'. La vista Inicio muestra fijados, frecuentes y recientes. Desanclar no borra la carpeta; los anclados se pueden reordenar; debe existir una opción para desactivar el seguimiento de recientes por privacidad.

> ℹ️ Es un sello de identidad de Windows 11. Para la estética objetivo conviene una sección 'Inicio/Acceso rápido' con fijados y recientes, y un control de privacidad para el historial de recientes/frecuentes.

## Pestañas (abrir, cerrar, reabrir, cambiar)

**Categoría:** Pestañas  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder

Navegación por pestañas dentro de una misma ventana para trabajar con varias carpetas, con apertura, cierre, reapertura y cambio rápido por teclado.

**Comportamiento esperado:** Ctrl+T abre una pestaña nueva (en la carpeta actual o en Inicio); Ctrl+W cierra la activa; Ctrl+Shift+T reabre la última cerrada donde esté soportado; Ctrl+Tab y Ctrl+Shift+Tab recorren las pestañas. Arrastrar reordena las pestañas y soltar archivos sobre una pestaña los mueve a esa carpeta. Cerrar la última pestaña cierra la ventana o deja una vacía; arrastrar una pestaña fuera crea una ventana nueva. El menú contextual de pestaña incluye 'Duplicar', 'Cerrar otras' y 'Cerrar las de la derecha'.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+T` | Nueva pestaña |
| `Ctrl+W` | Cerrar la pestaña activa |
| `Ctrl+Shift+T` | Reabrir la última pestaña cerrada (Dolphin, Nautilus; el Explorador de Windows 11 no lo soporta) |
| `Ctrl+Tab` | Ir a la siguiente pestaña (Ctrl+Shift+Tab a la anterior); en macOS Finder también Cmd+Shift+] y Cmd+Shift+[ |
| `Ctrl+1..9` | Ir a la pestaña n en gestores con pestañas numeradas (en macOS Finder, Cmd+1..4 cambian el MODO DE VISTA, no la pestaña) |

> ℹ️ Windows 11 añadió pestañas en 2022 (con Ctrl+T/Ctrl+W/Ctrl+Tab), pero no incorporó reabrir la última pestaña cerrada. Corrección de grounding: en macOS Finder, Cmd+1/2/3/4 cambian el modo de vista (iconos/lista/columnas/galería), NO saltan de pestaña; el cambio de pestaña en Finder es Ctrl+Tab o Cmd+Shift+[ / Cmd+Shift+]. Soltar archivos sobre una pestaña para moverlos entre carpetas es muy valorado por los power-users.

## Paneles duales / Vista dividida (F3)

**Categoría:** Diseño de paneles  
**Prioridad:** 🟡 Recomendable  
**Visto en:** KDE Dolphin, Cinnamon Nemo, Total Commander, Directory Opus

Mostrar dos paneles de carpetas lado a lado para copiar y mover entre ambos con comodidad, base del flujo de los gestores tipo comandante.

**Comportamiento esperado:** F3 divide la vista en dos paneles independientes, cada uno con su propia ruta, pestañas y selección; el panel activo se resalta y Tab cambia el foco entre ambos. Copiar y mover usan el otro panel como destino por defecto (en estilo comandante, F5 copia y F6 mueve al panel opuesto). Debe permitir opcionalmente sincronizar la navegación o igualar rutas entre paneles, y cerrar la división pulsando F3 de nuevo.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `F3` | Activar/desactivar la vista dividida (Dolphin, Nemo) |
| `Tab` | Cambiar el foco entre paneles |
| `F5` | Copiar al otro panel (estilo comandante: Total Commander, Directory Opus) |
| `F6` | Mover al otro panel (estilo comandante) |

> ℹ️ Windows 11 File Explorer y macOS Finder no traen paneles duales de fábrica; añadirlo es un plus muy pedido por power-users. Nota: F5/F6 como copiar/mover es específico de los gestores tipo comandante; en Explorer/Nautilus/Dolphin F5 es 'Actualizar', por lo que conviene documentar el conflicto y hacerlo configurable.

## Montar y expulsar unidades

**Categoría:** Unidades  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder

Montar y desmontar/expulsar de forma segura unidades extraíbles, particiones, imágenes y recursos de red desde la barra lateral o el menú contextual.

**Comportamiento esperado:** Las unidades aparecen en la barra lateral con un botón de expulsar; 'Expulsar' desmonta y avisa cuando es seguro retirar el dispositivo; montar una partición o imagen ISO la hace navegable. Debe impedir la expulsión mientras haya operaciones en curso y mostrar un mensaje claro si hay archivos abiertos. Las unidades cifradas piden contraseña al montar; los recursos de red se desconectan de forma equivalente. Debe mostrar el progreso del vaciado de caché antes de declarar que es seguro retirar el dispositivo.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Cmd+E` | Expulsar en macOS Finder |

> ℹ️ En Linux 'expulsar' (apagar la unidad) y 'desmontar con seguridad' pueden diferir. En Windows la expulsión también está en la bandeja del sistema ('Quitar hardware de forma segura').

## Acciones y scripts personalizados

**Categoría:** Extensibilidad  
**Prioridad:** 🟡 Recomendable  
**Visto en:** KDE Dolphin, GNOME Files (Nautilus), Cinnamon Nemo, macOS Finder, Windows 11 File Explorer, Directory Opus

Extender el menú contextual con entradas propias del usuario: menús de servicio, scripts, acciones y acciones rápidas que reciben la selección.

**Comportamiento esperado:** El menú contextual es extensible con entradas propias: menús de servicio de Dolphin (archivos .desktop en servicemenus, instalables desde la tienda de KDE), scripts de Nautilus (ejecutables en ~/.local/share/nautilus/scripts que aparecen bajo un submenú 'Scripts' y reciben la selección por variables de entorno), acciones de Nemo (archivos .nemo_action con condiciones por tipo MIME/extensión y número de elementos), Acciones rápidas y menú Servicios de Finder (creadas con Automator/Atajos), y controladores del menú contextual y carpeta 'SendTo' de Windows. Las entradas pueden filtrarse por tipo de archivo, número de elementos o ubicación, y reciben las rutas seleccionadas. Casos borde: una acción que no aplica al tipo o a la selección se oculta; los scripts se ejecutan con los permisos del usuario y conviene advertir antes de correr scripts de terceros.

> ℹ️ La extensibilidad del menú contextual es un rasgo esperado por power-users. Para un explorador nuevo conviene definir un formato de acción declarativo (condiciones por MIME/extensión, plantilla de comando con marcadores de posición) similar a los .nemo_action o a los menús de servicio de KDE.

## Ejecutar como administrador / Abrir como root

**Categoría:** Menú contextual  
**Prioridad:** ⚪ Opcional  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo

Ejecutar un programa con privilegios elevados o abrir una carpeta o archivo con permisos de administrador desde el menú contextual.

**Comportamiento esperado:** 'Ejecutar como administrador' lanza el ejecutable con elevación (UAC en Windows, pkexec/Polkit en Linux) tras confirmar; ciertas carpetas del sistema pueden ofrecer 'Abrir como administrador' o 'Editar como administrador'. Debe pedir siempre autenticación y dejar claro que se opera en un contexto elevado. Las acciones no elevables se deshabilitan y se advierte del riesgo de operar como root.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+Shift+Enter` | Ejecutar el elemento seleccionado como administrador (Windows) |

> ℹ️ En los sistemas Linux modernos se evita ejecutar el gestor completo como root; se prefiere elevar acciones puntuales con Polkit. Manejar con cuidado por seguridad y no ofrecerlo de forma indiscriminada.

