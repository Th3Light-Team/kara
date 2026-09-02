# 2. Selección, portapapeles y manipulación

> Especificación de *conveniencias* (qué y cómo se comporta), agnóstica de implementación.

Especificación exhaustiva de conveniencias de cara al usuario para seleccionar elementos, operar con el portapapeles, manipular archivos/carpetas y eliminarlos en un explorador moderno con estética y comodidades tipo Windows 11 sobre Linux. Cubre selección (todo/ninguno/invertir, por patrón/máscara, rubber-band, Ctrl+clic, Shift+clic, casillas, type-ahead), portapapeles (cortar/copiar/pegar, copiar a/mover a, duplicar, copiar como ruta), arrastrar y soltar con distinción mover vs copiar y entre paneles/pestañas, creación de accesos directos/enlaces, eliminación a papelera y borrado permanente, renombrado (F2 inline, secuencial con Tab, por lotes), creación de carpeta/archivo y deshacer/rehacer. Cada conveniencia está fundamentada en al menos un explorador real (Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files/Nautilus, macOS Finder, Directory Opus, Total Commander) con atajos precisos y variantes anotadas, para que quien implemente conozca el objetivo de UX y los casos borde. Esta revisión corrige atribuciones dudosas (encadenado de renombrado con Tab), añade eliminación/papelera, selección por patrón y creación de enlaces, y precisa conductas y casos borde verificables.

---

## Seleccionar todo

**Categoría:** Selección  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Total Commander

Selecciona todos los elementos del listado/carpeta actual con una sola acción.

**Comportamiento esperado:** Ctrl+A marca todos los archivos y carpetas visibles del panel actual y actualiza el contador de la barra de estado (p. ej. '25 elementos seleccionados'). Si el foco está dentro de un campo de renombrado inline o de la barra de dirección/búsqueda, Ctrl+A debe seleccionar el texto de ese campo, no los archivos. Con un filtro o búsqueda activa debe seleccionar solo lo que cumple el filtro/está visible, no el contenido oculto.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+A` | Con el foco en el panel de archivos, sin edición de texto activa. macOS Finder: Cmd+A. Total Commander también con Ctrl+Num + |

> ℹ️ Directory Opus y Total Commander añaden además 'seleccionar por patrón/máscara' (p. ej. *.jpg) como complemento del 'seleccionar todo'; ver la conveniencia 'select-by-pattern'.

## Deseleccionar todo / quitar selección

**Categoría:** Selección  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Total Commander

Vacía la selección actual sin abrir ni modificar ningún elemento.

**Comportamiento esperado:** Un clic en un área vacía del panel deselecciona todo (gesto universal en todos los gestores). Esc, cuando hay un renombrado o un rubber-band en curso, los cancela primero; si no hay ninguna operación en curso, quita la selección. Debe conservar el 'elemento con foco' (cursor) para que la navegación posterior por teclado siga desde ahí. Debe actualizar el contador de la barra de estado a 0 seleccionados.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Esc` | Cancela el renombrado o el arrastre/marco en curso; si no hay ninguno, quita la selección |
| `Ctrl+Num -` | Total Commander: deseleccionar todo (numpad) |

> ℹ️ El gesto fundamentado en todos los gestores es el clic en zona vacía. Que Esc quite la selección (cuando no cancela otra cosa) no es universal: en Windows 11 File Explorer 'Seleccionar ninguno' está en la barra de comandos/cinta sin atajo de teclado por defecto, y Esc se usa sobre todo para cancelar operaciones en curso. Recomendable adoptar el comportamiento combinado descrito como objetivo de UX.

## Invertir selección

**Categoría:** Selección  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), Directory Opus, Total Commander

Intercambia el estado de selección: lo seleccionado pasa a no seleccionado y viceversa.

**Comportamiento esperado:** Sirve para el patrón 'seleccionar todo menos unos pocos': el usuario marca los elementos a excluir y luego invierte. Debe operar solo sobre los elementos visibles/filtrados (no toca lo oculto por filtro o búsqueda), preservar el elemento con foco y actualizar el contador de la barra de estado. No debe abrir ni modificar ningún elemento.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+Shift+A` | KDE Dolphin: 'Invertir selección' |
| `Num *` | Total Commander: el asterisco del teclado numérico invierte la selección |
| `Ctrl+Shift+I` | GNOME Files (Nautilus), históricamente: 'Invertir selección' |

> ℹ️ Windows 11 la ofrece en la cinta/menú 'Invertir selección' pero sin atajo de teclado por defecto. GNOME Files usa históricamente Ctrl+Shift+I (algunas versiones recientes la han retirado del menú). Nemo la incluye en el menú Edición sin atajo fijo. macOS Finder no ofrece invertir selección de forma nativa (por eso no aparece en seenIn).

## Selección con rubber-band (marco elástico)

**Categoría:** Selección  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Total Commander

Arrastrar desde un espacio vacío dibuja un rectángulo elástico que selecciona todos los elementos que toca.

**Comportamiento esperado:** El marco debe iniciarse solo desde zona vacía (nunca sobre un ítem, para no confundirlo con arrastrar-mover). En vista de iconos selecciona los elementos que el rectángulo interseca; en vista de detalles/lista selecciona por filas. Mantener Ctrl (Cmd en macOS) mientras se dibuja el marco añade a la selección existente en lugar de reemplazarla. Al llegar al borde del panel debe hacer autoscroll continuo. Feedback visual: rectángulo semitransparente con borde. Soltar fija la selección y actualiza el contador; Esc durante el marco lo cancela sin cambiar la selección previa.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl (mantener durante el marco)` | Suma el nuevo marco a la selección previa en vez de reemplazarla. macOS Finder: Cmd |

> ℹ️ Gotcha clásico: si el marco puede iniciarse encima de un ítem se dispara un arrastre-mover accidental; delimitar bien la zona de inicio del rubber-band es clave. En vista de detalles, decidir si solo la primera columna (nombre) o toda la fila inicia el marco.

## Ctrl+clic (alternar selección individual)

**Categoría:** Selección  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Total Commander

Añade o quita un único elemento de la selección sin afectar al resto, permitiendo selecciones discontinuas.

**Comportamiento esperado:** Ctrl+clic sobre un ítem no seleccionado lo añade; sobre uno ya seleccionado lo quita. No debe abrir el elemento ni perder el resto de la selección. Combinable con Shift+clic para acumular varios rangos. Por teclado, el ítem con foco se puede alternar sin moverlo con Ctrl+Espacio. El 'ancla' para rangos futuros pasa a ser el último elemento pulsado con Ctrl.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+Click` | macOS Finder: Cmd+Click |
| `Ctrl+Space` | Alterna la selección del ítem con foco sin desplazar el foco |

> ℹ️ El 'ancla' para rangos futuros debe actualizarse al último elemento pulsado con Ctrl. En Total Commander existe además el modelo clásico de teclado: Insert marca el ítem actual y baja una fila, y Espacio marca mostrando el tamaño de carpeta.

## Shift+clic (selección de rango contiguo)

**Categoría:** Selección  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Total Commander

Selecciona todos los elementos comprendidos entre el ancla actual y el elemento pulsado.

**Comportamiento esperado:** El primer clic fija un 'ancla'; Shift+clic selecciona el rango contiguo desde el ancla hasta el destino, reemplazando la selección previa salvo que se combine con Ctrl. Volver a hacer Shift+clic recalcula el rango desde el mismo ancla (no acumula ni crece indefinidamente). Shift+flechas y Shift+Inicio/Fin extienden el rango por teclado; Ctrl+Shift+clic añade un segundo rango sin perder el anterior. El ancla solo debe cambiar con un clic simple o con Ctrl+clic, nunca con Shift+clic.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Shift+Click` | Selecciona del ancla al elemento pulsado |
| `Shift+Arrow / Shift+Home / Shift+End` | Extiende el rango por teclado |
| `Ctrl+Shift+Click` | Añade un segundo rango contiguo conservando la selección previa |

> ℹ️ Gotcha: si el ancla no se preserva correctamente, Shift+clic sucesivos producen rangos incoherentes; el ancla solo debe cambiar con clic simple o Ctrl+clic.

## Seleccionar por patrón/máscara

**Categoría:** Selección  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Directory Opus, Total Commander

Seleccionar (o deseleccionar) de golpe los elementos cuyo nombre coincide con un patrón de comodines, p. ej. *.jpg.

**Comportamiento esperado:** Abre un pequeño cuadro donde el usuario escribe una máscara con comodines (* y ?) y marca todos los elementos coincidentes del panel actual; una variante paralela deselecciona los coincidentes. Debe operar solo sobre lo visible/filtrado, ser insensible a mayúsculas (idealmente configurable), poder acumularse sobre la selección previa en el modo 'añadir' y actualizar el contador de estado. Útil para 'seleccionar todos los .tmp' o 'quitar todos los .bak' sin marcarlos a mano.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Num +` | Total Commander: seleccionar grupo por máscara (numpad) |
| `Num -` | Total Commander: deseleccionar grupo por máscara (numpad) |

> ℹ️ Correcta atribución: Windows 11 File Explorer, KDE Dolphin, GNOME Files (Nautilus), Cinnamon Nemo y macOS Finder NO ofrecen 'seleccionar por patrón' nativo; es un rasgo distintivo de gestores orientados a potencia (Total Commander con las teclas del numpad; Directory Opus en Edit > Select > por patrón/avanzado). Recomendable incluirlo por su gran valor para usuarios avanzados y como complemento de 'seleccionar todo'.

## Casillas de selección de elementos

**Categoría:** Selección  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Directory Opus

Casillas que aparecen sobre cada elemento para seleccionar con un solo clic, sin teclas modificadoras.

**Comportamiento esperado:** Al pasar el ratón sobre un ítem aparece una casilla en su esquina; marcarla lo añade a una selección múltiple discontinua sin necesidad de Ctrl. En vista de detalles se muestra una casilla de 'seleccionar todo' en el encabezado. Evita perder la selección por un clic accidental y facilita el uso táctil/con lápiz. Debe poder activarse/desactivarse desde el menú de vista ('Ver > Mostrar > Casillas de elementos' en Windows).

> ℹ️ Windows 11: 'Ver > Mostrar > Casillas de elementos'. KDE Dolphin muestra marcadores +/- al pasar el ratón que cumplen la misma función de alternar la selección sin modificadores. Directory Opus tiene un 'checkbox mode' conmutable. macOS Finder, GNOME Files (Nautilus) y Cinnamon Nemo no ofrecen casillas por defecto.

## Selección por escritura (type-ahead)

**Categoría:** Selección  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Total Commander

Al teclear con el foco en el listado, salta y selecciona el primer elemento cuyo nombre empieza por lo escrito.

**Comportamiento esperado:** Escribir 'in' selecciona el primer archivo que empieza por 'in'; seguir tecleando afina la coincidencia acumulando el buffer; tras una breve pausa (aprox. 1 s) el buffer se reinicia para empezar una nueva búsqueda. Debe respetar el orden de ordenación actual y ser insensible a mayúsculas/acentos. No debe confundirse con la búsqueda/filtro global de la carpeta, que tiene su propio atajo (p. ej. Ctrl+F).

**Atajos:**

| Atajo | Contexto |
|---|---|
| `(teclear el inicio del nombre)` | Con el foco en el panel de archivos, sin edición activa |

> ℹ️ Grounding matizado: GNOME Files (Nautilus) convierte el tecleo en una búsqueda incremental recursiva con resaltado (comportamiento controvertido) en lugar de un simple salto; Windows 11, Dolphin, Nemo y Finder saltan al elemento coincidente. Total Commander permite alternar entre 'saltar al nombre' y 'filtro rápido'.

## Cortar, copiar y pegar

**Categoría:** Portapapeles  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Total Commander

Operaciones de portapapeles para mover (cortar+pegar) o copiar (copiar+pegar) archivos y carpetas.

**Comportamiento esperado:** Ctrl+X marca para mover y atenúa visualmente (fade) los ítems cortados hasta que se pegan; Ctrl+C copia; Ctrl+V pega en la carpeta activa. Pegar tras copiar en la misma carpeta genera un duplicado con sufijo ('archivo - copia'). Cortar+pegar en la misma carpeta no debe hacer nada. Si el usuario copia otra cosa o pulsa Esc, la marca de 'cortado' se cancela y los ítems vuelven a su aspecto normal. Debe resolver colisiones de nombre con diálogo (Reemplazar / Omitir / Conservar ambos) y mostrar barra de progreso, velocidad, tiempo restante y opción de cancelar en operaciones largas. Debe aceptar archivos copiados desde otras ventanas, el escritorio u otras apps mediante el portapapeles del sistema.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+X` | Cortar (marcar para mover) |
| `Ctrl+C` | Copiar |
| `Ctrl+V` | Pegar en la carpeta activa |

> ℹ️ macOS Finder no tiene 'cortar' clásico: se copia con Cmd+C y se 'mueve aquí' al pegar con Cmd+Option+V. El resto de gestores usan cortar/copiar/pegar estándar. Recomendable atenuar los ítems cortados para dar feedback del estado pendiente.

## Copiar a / Mover a (elegir carpeta destino)

**Categoría:** Portapapeles  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), Directory Opus, Total Commander

Comando para copiar o mover la selección a una carpeta elegida en un diálogo, sin arrastrar ni usar el portapapeles.

**Comportamiento esperado:** Abre un selector de carpetas (con destinos recientes y favoritos y opción de crear carpeta nueva en el propio diálogo) y ejecuta la operación al confirmar. Es útil cuando origen y destino no caben a la vez en pantalla o para operaciones de una sola mano. Debe aplicar las mismas reglas de colisión, progreso y cancelación que pegar, y dejar la selección resultante en el destino cuando sea visible.

> ℹ️ Windows 11: 'Copiar a'/'Mover a' en la barra de comandos y en el menú contextual. Total Commander y Directory Opus lo resuelven con F5 (copiar) y F6 (mover) hacia el panel opuesto en su modelo de doble panel.

## Duplicar

**Categoría:** Portapapeles  
**Prioridad:** ⚪ Opcional  
**Visto en:** KDE Dolphin, macOS Finder, Directory Opus

Crear una copia del elemento seleccionado en la misma carpeta en un solo paso.

**Comportamiento esperado:** Genera una copia con sufijo ('archivo - copia' / 'archivo (copia)') sin tocar el portapapeles del sistema (no interfiere con lo que el usuario tuviera copiado). Debe seleccionar el nuevo elemento y, opcionalmente, dejarlo listo para renombrar. Con varios seleccionados, duplica todos respetando la numeración de sufijos.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+D` | KDE Dolphin: 'Duplicar aquí'. macOS Finder: Cmd+D |

> ℹ️ Windows 11 no tiene comando 'Duplicar' ni atajo directo: se logra con Ctrl+C y Ctrl+V en la misma carpeta (genera 'archivo - copia'). GNOME Files y Nemo tampoco lo traen por defecto. Para el objetivo tipo Windows conviene añadir un 'Duplicar' explícito como mejora de comodidad, ya que no ensucia el portapapeles.

## Copiar como ruta

**Categoría:** Rutas  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, Directory Opus

Copiar al portapapeles la ruta completa de los elementos seleccionados como texto plano.

**Comportamiento esperado:** Coloca la ruta absoluta en el portapapeles de texto (entrecomillada si contiene espacios), lista para pegar en una terminal, un diálogo 'Abrir' o un documento. Con varios elementos seleccionados, copia una ruta por línea. No debe copiar el archivo en sí, solo el texto de la ruta. Conviene mantenerla separada de 'copiar nombre de archivo' y ofrecer variantes.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+Shift+C` | Windows 11: 'Copiar como ruta de acceso' (atajo directo desde Windows 11). También con Shift+clic derecho > 'Copiar como ruta' |

> ℹ️ KDE Dolphin ofrece 'Copiar ubicación'. Nemo tiene 'Copiar como ruta'. GNOME Files (Nautilus) no la trae de serie en el menú contextual (se puede ver/copiar la ruta con Ctrl+L en la barra de dirección), por eso no está en seenIn. Útil ofrecer variantes: ruta completa, ruta de la carpeta contenedora y solo el nombre.

## Mover vs copiar al arrastrar (modificadores)

**Categoría:** Arrastrar y soltar  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Total Commander

El resultado de soltar depende del destino y de las teclas modificadoras: mover, copiar o crear acceso directo/enlace.

**Comportamiento esperado:** Regla por defecto: arrastrar dentro del mismo volumen mueve; a otro volumen/disco copia. Mantener Ctrl fuerza copiar; Shift fuerza mover; Alt (o Ctrl+Shift) en Windows crea un acceso directo. El cursor debe indicar la acción resultante (+ para copiar, flecha para mover, símbolo de enlace/atajo para acceso directo, símbolo de prohibido en destinos inválidos). La carpeta bajo el cursor debe resaltarse; al mantenerse sobre una carpeta debe expandirse/abrirse tras una breve pausa (spring-loaded). Soltar con el botón derecho debe abrir un menú 'Copiar aquí / Mover aquí / Crear acceso directo / Cancelar'.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl (arrastrando)` | Forzar copiar |
| `Shift (arrastrando)` | Forzar mover |
| `Alt (arrastrando)` | Windows: crear acceso directo (también Ctrl+Shift) |
| `Cmd / Option / Cmd+Option (arrastrando)` | macOS Finder: Cmd fuerza mover, Option fuerza copiar, Cmd+Option crea alias |

> ℹ️ En macOS los modificadores cambian (Cmd=mover, Option=copiar, Cmd+Option=alias). En Dolphin/Nautilus/Nemo, soltar sin modificador suele mostrar un menú Copiar/Mover/Enlazar. Gotcha: distinguir claramente mismo-volumen (mover) de distinto-volumen (copiar) para no sorprender al usuario.

## Arrastrar y soltar entre paneles/pestañas

**Categoría:** Arrastrar y soltar  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), Directory Opus, Total Commander

Mover o copiar archivos arrastrándolos entre dos paneles (vista dividida), hacia otra pestaña, ventana o al árbol lateral.

**Comportamiento esperado:** En vista de doble panel, arrastrar del panel activo al otro mueve o copia según las reglas de modificadores (ver 'move-vs-copy-drag'). Al arrastrar sobre la pestaña de una pestaña inactiva, esta debe activarse tras una breve pausa (spring-loaded tabs) para poder soltar dentro de ella. Debe funcionar también hacia el árbol de carpetas lateral y hacia carpetas anidadas (que se expanden al mantener el cursor encima). Los destinos válidos se resaltan; los inválidos muestran cursor de 'prohibido'.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `F3` | KDE Dolphin: activar/desactivar la vista dividida (doble panel) |

> ℹ️ Total Commander y Directory Opus son de doble panel nativo (F5 copia y F6 mueve al panel opuesto). Windows 11 permite arrastrar entre pestañas y ventanas, pero su vista dividida nativa es limitada. Dolphin y Nemo tienen panel dividido (Dolphin con F3; Nemo con F3 también).

## Crear acceso directo / enlace

**Categoría:** Manipulación  
**Prioridad:** ⚪ Opcional  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus

Crear un acceso directo (Windows), un enlace simbólico (Linux) o un alias (macOS) a los elementos seleccionados, sin duplicar los datos.

**Comportamiento esperado:** Desde el menú contextual 'Crear acceso directo' (o 'Enlazar aquí' al soltar) genera un puntero al original en la carpeta actual. En Linux debe permitir distinguir enlace simbólico de enlace duro; un enlace roto (destino inexistente) debe mostrarse con un indicador visual (icono con emblema de rotura). Arrastrar con el modificador de enlace crea el enlace directamente; soltar con botón derecho ofrece 'Crear acceso directo / Enlazar aquí' en el menú. El nombre por defecto añade sufijo tipo 'archivo - acceso directo' / 'Enlace hacia archivo'.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+Shift+Arrastrar` | Windows y varios gestores Linux: crear enlace/acceso directo al soltar. En Windows también con Alt+Arrastrar |
| `Cmd+Option+Arrastrar` | macOS Finder: crear alias al arrastrar. Menú/atajo: Cmd+L ('Crear alias') |

> ℹ️ Windows: 'Crear acceso directo' (.lnk). macOS: alias (Cmd+Option+arrastrar, o menú 'Crear alias' / Cmd+L). Dolphin/Nemo/Nautilus: 'Enlazar aquí' al soltar y opción de enlace simbólico. Total Commander no ofrece un atajo simple de serie para esto, por eso no aparece en seenIn. Redondea el bloque de 'manipulación' junto a copiar/mover/duplicar.

## Renombrar inline (F2)

**Categoría:** Renombrado  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Total Commander

Editar el nombre del elemento seleccionado directamente sobre su etiqueta.

**Comportamiento esperado:** F2 abre un campo de edición sobre el nombre con el texto ya seleccionado. Por defecto debe seleccionar solo el nombre base y dejar fuera la extensión, para no borrarla por accidente. Enter confirma; Esc cancela y restaura el nombre original. Debe validar caracteres no permitidos, nombres duplicados y nombres reservados, avisando (borde/tooltip de error) sin cerrar el editor. Un clic-pausa-clic sobre un elemento ya seleccionado también inicia el renombrado inline.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `F2` | Windows 11, Dolphin, Nemo, Nautilus. macOS Finder: Enter/Return inicia el renombrado |

> ℹ️ En macOS la semántica se invierte respecto a Windows: Enter renombra y Cmd+O abre. Windows y Dolphin permiten configurar si la selección inicial incluye o no la extensión.

## Renombrado secuencial con Tab

**Categoría:** Renombrado  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer

Encadenar el renombrado de varios elementos uno tras otro sin volver a pulsar F2 en cada uno.

**Comportamiento esperado:** Durante un renombrado inline, Tab confirma el nombre actual y abre inmediatamente el editor del siguiente elemento del listado; Shift+Tab retrocede al anterior. Debe seguir el orden de ordenación visible y hacer autoscroll para mantener a la vista el elemento en edición. Esc cancela la cadena dejando confirmados los nombres ya validados.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Tab` | Confirma el nombre actual y salta al siguiente elemento |
| `Shift+Tab` | Confirma y salta al elemento anterior |

> ℹ️ CORRECCIÓN DE GROUNDING: este encadenado con Tab está fundamentado con solidez solo en Windows 11 File Explorer (comportamiento clásico muy valorado por usuarios avanzados). Se retiró KDE Dolphin de seenIn porque no se confirma que encadene renombrados con Tab por defecto. GNOME Files (Nautilus) y Nemo no encadenan con Tab; para renombrar en serie ofrecen su diálogo de renombrado por lotes (ver 'batch-rename'). Recomendable implementarlo como diferenciador tipo Windows.

## Renombrado por lotes

**Categoría:** Renombrado  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, GNOME Files (Nautilus), Directory Opus, Total Commander

Renombrar muchos elementos a la vez mediante un patrón, numeración automática o buscar/reemplazar.

**Comportamiento esperado:** Con varios elementos seleccionados y F2, el gestor aplica un nombre base común y numera automáticamente: 'foto (1)', 'foto (2)'... Los gestores avanzados abren un diálogo con vista previa en vivo, buscar y reemplazar, numeración con relleno de ceros, cambio de mayúsculas/minúsculas y uso de metadatos (fecha, dimensiones). Debe mostrar previsualización antes de aplicar, avisar de colisiones y ser reversible con Deshacer.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `F2` | Windows: nombre base + (n). Dolphin/Nautilus: abre el diálogo de renombrado por lotes con varios seleccionados |

> ℹ️ Dolphin usa '#' como marcador de posición para la numeración. GNOME Files trae un diálogo con 'buscar y reemplazar' y 'numeración' (con formato). En Windows la herramienta avanzada es PowerRename (PowerToys), no nativa del Explorador. Total Commander y Directory Opus tienen 'Multi-Rename Tool' muy potentes con expresiones. Cinnamon Nemo no incorpora un diálogo de renombrado por lotes de serie (a menudo se integra con una acción/herramienta externa), por eso no está en seenIn.

## Crear carpeta nueva

**Categoría:** Creación  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Total Commander

Crear una carpeta vacía en la ubicación actual y entrar de inmediato en modo renombrado.

**Comportamiento esperado:** Ctrl+Shift+N crea 'Nueva carpeta' y abre directamente el editor de nombre con el texto seleccionado. Si ya existe 'Nueva carpeta', debe añadir sufijo ('Nueva carpeta (2)'). La carpeta debe crearse en la ubicación visible, quedar seleccionada y visible (autoscroll). Enter confirma el nombre; Esc cancela y elimina la carpeta recién creada si el nombre no se ha confirmado.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+Shift+N` | Windows 11, GNOME Files, Nemo. macOS Finder: Cmd+Shift+N |
| `F10` | KDE Dolphin: menú 'Crear nuevo > Carpeta' |
| `F7` | Total Commander: crear carpeta |

> ℹ️ Dolphin usa F10 por defecto para el menú 'Crear nuevo'; el resto de gestores gráficos usan Ctrl+Shift+N (Cmd+Shift+N en macOS). Total Commander usa F7 para crear carpeta directamente.

## Crear archivo nuevo (menú Nuevo)

**Categoría:** Creación  
**Prioridad:** 🟡 Recomendable  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, Directory Opus

Crear un archivo vacío o a partir de una plantilla (documento de texto, hoja, etc.) desde el menú contextual.

**Comportamiento esperado:** El submenú 'Nuevo' lista los tipos disponibles según las plantillas instaladas. Crea el archivo en la carpeta actual y entra en renombrado inmediato. Debe manejar colisiones de nombre con sufijo. Como mínimo debería permitir crear un documento de texto vacío de serie.

> ℹ️ Grounding matizado: tanto GNOME Files (Nautilus) como Cinnamon Nemo pueblan el submenú 'Nuevo documento' a partir de plantillas colocadas en ~/Plantillas (~/Templates); sin plantillas pueden no ofrecer 'archivo vacío'. Windows rellena el submenú 'Nuevo' según las aplicaciones instaladas, con 'Documento de texto' de serie. KDE Dolphin ('Crear nuevo') y Directory Opus permiten archivo vacío/plantilla. Conviene incluir al menos 'Documento de texto' por defecto.

## Eliminar a la papelera y borrado permanente

**Categoría:** Eliminación  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus, Total Commander

Enviar los elementos seleccionados a la papelera (reversible) o borrarlos de forma permanente.

**Comportamiento esperado:** Supr (Delete) mueve la selección a la papelera del sistema, actualiza el contador de estado y es reversible con Ctrl+Z y desde la propia papelera. Shift+Supr borra de forma permanente saltándose la papelera y debe pedir confirmación explícita porque no es reversible. Tras eliminar debe quedar seleccionado automáticamente el elemento adyacente (siguiente, o el anterior si era el último) para no perder el punto de foco de teclado. Debe mostrar diálogo de progreso/cancelación en borrados masivos y, cuando un elemento no pueda ir a la papelera (volumen de red, unidad extraíble o archivo demasiado grande), avisar y ofrecer borrado permanente.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Supr (Delete)` | Enviar a la papelera. macOS Finder: Cmd+Supr (Cmd+Delete) |
| `Shift+Supr` | Borrado permanente con confirmación. macOS Finder: Cmd+Shift+Supr vacía la papelera |
| `F8` | Total Commander: eliminar (papelera si está configurada); Shift+Del fuerza permanente |

> ℹ️ Conveniencia fundamental del dominio 'manipulación' que faltaba en el borrador. macOS Finder usa Cmd+Supr para 'Mover a la papelera' (la tecla Supr sola no borra) y Cmd+Shift+Supr para vaciar. Total Commander usa F8 o Supr. Windows 11 pide confirmación al enviar a la papelera solo si está configurada; el borrado permanente (Shift+Supr) siempre debe confirmar. Coherente con 'undo-redo', que revierte el envío a la papelera pero NO el borrado permanente ni el vaciado.

## Deshacer y rehacer operaciones

**Categoría:** Deshacer/Rehacer  
**Prioridad:** 🔴 Imprescindible  
**Visto en:** Windows 11 File Explorer, KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus

Revertir o rehacer la última operación de archivos: mover, copiar, renombrar, crear o enviar a la papelera.

**Comportamiento esperado:** Ctrl+Z deshace la última operación: deshacer un movimiento devuelve el elemento a su origen, deshacer un renombrado restaura el nombre anterior y deshacer un envío a la papelera lo restaura a su ubicación. Ctrl+Y (o Ctrl+Shift+Z) rehace. Debe mantener una pila con varios niveles y describir la acción concreta en el menú ('Deshacer Renombrar', 'Deshacer Mover'). El borrado permanente (Shift+Supr) y el vaciado de papelera no son reversibles y deben advertirlo claramente antes de ejecutarse.

**Atajos:**

| Atajo | Contexto |
|---|---|
| `Ctrl+Z` | Deshacer la última operación de archivos. macOS Finder: Cmd+Z |
| `Ctrl+Y` | Rehacer (Windows). GNOME Files, Dolphin y Nemo: Ctrl+Shift+Z. macOS Finder: Cmd+Shift+Z |

> ℹ️ Se añadió macOS Finder a seenIn: soporta Cmd+Z para deshacer (mover, renombrar, copiar, mover a la papelera) y Cmd+Shift+Z para rehacer. Nautilus, Nemo y Dolphin usan Ctrl+Shift+Z para rehacer. Total Commander no ofrece una pila de deshacer general de operaciones de archivo por defecto (por eso no está en seenIn). El alcance y la profundidad de la pila varían; conviene que abarque copiar/mover/renombrar/crear/papelera como mínimo.

