# Filosofía de las conveniencias

## Qué buscamos: la "sensación" de un buen explorador

Un explorador de archivos no se juzga por su lista de funciones, sino por la **fricción acumulada** de mil gestos diarios: subir un nivel, saltar a una carpeta hermana, seleccionar todo menos tres archivos, mover algo al otro panel, deshacer un borrado. Este paquete describe las *conveniencias* — no las capacidades técnicas, sino los comportamientos de interfaz que hacen que esos gestos sean rápidos, predecibles y reversibles.

El objetivo es concreto: **la estética y las comodidades del Explorador de Windows 11, sobre Linux**, con su modelo de permisos POSIX y sus convenciones (papelera FreeDesktop, MIME, miniaturas de freedesktop). Adoptamos el atajo de Windows como principal y anotamos las variantes de KDE Dolphin, Cinnamon Nemo, GNOME Files (Nautilus), macOS Finder, Directory Opus y Total Commander.

## Principios de diseño

- **Fundamentado (grounded), no inventado.** Cada conveniencia y cada atajo existe en al menos un explorador real. Las revisiones adversariales corrigieron atribuciones falsas (p. ej. Alt+Home no es de Windows; F5 = copiar en comandantes, no refrescar).
- **Reversibilidad por defecto.** Eliminar va a la papelera; mover/copiar/renombrar/crear entran en la pila de Deshacer. Lo irreversible (Shift+Supr, vaciar papelera) **siempre** confirma y el foco recae en el botón seguro.
- **Feedback proporcional.** Nada de operaciones silenciosas: fase de "Calculando…", progreso con velocidad/ETA/cancelar, resolución de conflictos clara (Reemplazar/Omitir/Conservar ambos), y resumen de errores sin abortar todo el lote.
- **El estado se recuerda.** Vista, orden, columnas y zoom persisten por carpeta; las pestañas y la sesión se restauran; el scroll y la selección vuelven al navegar Atrás.
- **Coherencia de conflictos de teclado.** Donde una tecla tiene dos tradiciones (Retroceso = Atrás en Windows vs Subir en KDE), se elige la convención Windows y se deja **configurable**.
- **Progresivo: casual y power-user.** Chips/menús para el casual; sintaxis, comodines, patrones y paneles duales para el avanzado — sin que uno estorbe al otro.
- **Privacidad y seguridad.** Interruptor para el historial de recientes/frecuentes; mostrar siempre la extensión real de ejecutables; elevar acciones puntuales con Polkit, nunca el explorador entero como root.

## Cómo leer esta especificación

La spec se divide en **seis dominios**: Navegación e historial; Selección, portapapeles y manipulación; Vistas, ordenación y organización; Búsqueda y filtrado; Operaciones de fichero y feedback; y Menú contextual/power-user. Cada dominio contiene *conveniencias* con esta anatomía:

1. **`behavior`** — el objetivo de UX y los **casos borde** verificables (qué pasa con rutas rotas, volúmenes desmontados, colisiones de nombre, árboles enormes). Es el corazón: describe *qué debe sentir* el usuario, sin imponer implementación.
2. **`shortcuts`** — teclas con su `context` por explorador.
3. **`seenIn`** — dónde está fundamentada.
4. **`priority`** — `must` / `should` / `could`, que alimenta el roadmap.
5. **`notes`** — variantes, conflictos y correcciones de grounding.

Los dos documentos que acompañan a este texto — el **mapa maestro de atajos** y el **roadmap priorizado** — están compilados desde esos campos: usa el keymap para decidir la asignación de teclas de un vistazo, y el roadmap para decidir el **orden de construcción**. Empieza siempre por los `must`: son la base sin la cual el explorador no se siente completo.
