// Paleta y métricas Fluent (Windows 11), en claro y oscuro.
//
// Es el único sitio donde hay colores escritos a mano: cualquier componente que
// necesite uno lo pide por nombre. Un `#F9F9F9` suelto en un Rectangle es un
// componente que no cambia de tema.
//
// El tema arranca siguiendo al sistema y el conmutador de la barra de título
// lo fija a mano a partir de ahí. "The system" is the Settings portal's
// `color-scheme` and `accent-color`, which GNOME and Plasma both publish and
// `Main.qml` feeds in live; only without a portal does Qt's palette decide,
// and outside Plasma that palette is always light.
pragma Singleton

import QtQuick

QtObject {
    id: theme

    // 0 = seguir al sistema, 1 = claro fijo, 2 = oscuro fijo.
    property int mode: 0

    property SystemPalette system: SystemPalette { colorGroup: SystemPalette.Active }

    // What the desktop said through the Settings portal: -1 nothing (no
    // portal), 0 no preference — light, on every desktop —, 1 dark, 2 light.
    property int desktopScheme: -1
    // The desktop's accent; fully transparent when it has none.
    property color desktopAccent: "transparent"
    readonly property bool hasDesktopAccent: theme.desktopAccent.a > 0

    // Without a portal, se deduce del brillo del fondo del sistema en vez de
    // leer un enum de esquema de color: el enum solo existe a partir de
    // ciertas versiones de Qt, el color de ventana lleva ahí desde siempre.
    readonly property bool systemIsDark: theme.desktopScheme >= 0 ? theme.desktopScheme === 1 : theme.system.window.hslLightness < 0.5
    readonly property bool dark: theme.mode === 0 ? theme.systemIsDark : theme.mode === 2

    function toggle() {
        theme.mode = theme.dark ? 1 : 2;
    }

    // An accent colour as text, rings and borders can use it: pulled darker
    // on a light window and lighter on a dark one, into the lightness bands
    // of Windows' own AccentDark1 and AccentLight2 — the Fluent defaults
    // above sit at 0.36 and 0.69. A desktop accent picked for GNOME's
    // headerbars (slate, #6f8396, is 0.51) is too faint for either as is.
    function readable(color: color): color {
        const lightness = theme.dark ? Math.max(color.hslLightness, 0.68) : Math.min(color.hslLightness, 0.40);
        // A grey has no hue (Qt reports -1): keep it a grey.
        return Qt.hsla(Math.max(0, color.hslHue), color.hslSaturation, lightness, 1);
    }

    // ---- Superficies -------------------------------------------------------
    readonly property color windowBg: theme.dark ? "#1F1F1F" : "#F3F3F3"
    readonly property color titleBar: theme.dark ? "#2B2B2B" : "#F9F9F9"
    readonly property color toolbar:  theme.dark ? "#2B2B2B" : "#F9F9F9"
    readonly property color sidebar:  theme.dark ? "#262626" : "#EBEBEB"
    readonly property color content:  theme.dark ? "#282828" : "#FFFFFF"
    readonly property color statusBar: theme.dark ? "#242424" : "#F0F0F0"
    // Fondo de los campos (dirección, búsqueda): en claro va más blanco que la
    // barra que los contiene, en oscuro más claro. En los dos casos, más cerca
    // del papel que del marco.
    readonly property color field:    theme.dark ? "#333333" : "#FFFFFF"

    // ---- Texto -------------------------------------------------------------
    readonly property color text:       theme.dark ? "#F3F3F3" : "#1A1A1A"
    readonly property color textDim:    theme.dark ? "#9C9C9C" : "#606060"
    readonly property color headerText: theme.dark ? "#B4B4B4" : "#5E5E5E"
    readonly property color textOnAccent: theme.hasDesktopAccent ? (theme.dark ? "#1A1A1A" : "#FFFFFF") : (theme.dark ? "#00293D" : "#FFFFFF")
    // El texto deshabilitado no basta con bajarle la opacidad: sobre el fondo
    // oscuro se convierte en un gris sucio distinto del de la maqueta.
    readonly property color textDisabled: theme.dark ? "#5D5D5D" : "#A0A0A0"

    // ---- Interacción -------------------------------------------------------
    readonly property color hover:     theme.dark ? "#383838" : "#E4E4E4"
    readonly property color pressed:   theme.dark ? "#303030" : "#DBDBDB"
    // With a desktop accent, the selection is that accent washed over the
    // content background, as Windows does with its own; without one, the
    // Fluent blues the spike was drawn with.
    readonly property color selection: theme.hasDesktopAccent ? Qt.tint(theme.content, Qt.alpha(theme.accent, theme.dark ? 0.32 : 0.18)) : (theme.dark ? "#34506B" : "#D6E8F9")
    readonly property color accent:    theme.hasDesktopAccent ? theme.readable(theme.desktopAccent) : (theme.dark ? "#60CDFF" : "#005FB8")
    readonly property color divider:   theme.dark ? "#383838" : "#E7E7E7"
    readonly property color danger:    "#C42B1C"
    readonly property color success:   theme.dark ? "#6CCB5F" : "#107C10"
    readonly property color dangerText: "#FFFFFF"

    // ---- Métricas ----------------------------------------------------------
    readonly property int radius: 4
    readonly property int radiusLarge: 8
    readonly property int titleBarHeight: 40
    readonly property int commandBarHeight: 48
    readonly property int statusBarHeight: 28
    readonly property int fieldHeight: 32
    readonly property int rowHeight: 32
    readonly property int gap: 8

    // ---- Tipografía --------------------------------------------------------
    // `font.family` admite **un** nombre: ni una lista ni una cadena con comas
    // (comprobado contra este Qt, que rechaza `font.families`). Así que la
    // cadena de respaldo se resuelve aquí, una vez, contra las fuentes que hay
    // instaladas de verdad. «Segoe UI» es la de Windows 11 y casi nunca estará;
    // el último recurso es la que Plasma tenga configurada, que siempre existe.
    readonly property string family: {
        const wanted = ["Segoe UI", "Selawik", "Inter", "Noto Sans"];
        const installed = Qt.fontFamilies();
        for (let i = 0; i < wanted.length; ++i) {
            if (installed.indexOf(wanted[i]) >= 0)
                return wanted[i];
        }
        // Vacío significa «la fuente por defecto de la aplicación», que es la
        // que Plasma ya ha elegido. Es mejor final que forzar una concreta.
        return "";
    }
    readonly property int sizeSmall: 11
    readonly property int sizeBase: 13
    readonly property int sizeTitle: 13
}
