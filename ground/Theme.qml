// Paleta Fluent / Windows 11 con modo claro y oscuro.
// Un solo objeto con propiedades derivadas de `dark`; se pasa a los componentes.
import QtQuick

QtObject {
    id: t
    property bool dark: false

    // Superficies
    readonly property color windowBg: dark ? "#1F1F1F" : "#F3F3F3"
    readonly property color titleBar: dark ? "#2B2B2B" : "#F9F9F9"
    readonly property color sidebar:  dark ? "#262626" : "#EBEBEB"
    readonly property color content:  dark ? "#282828" : "#FFFFFF"
    readonly property color toolbar:  dark ? "#2B2B2B" : "#F9F9F9"

    // Texto
    readonly property color text:       dark ? "#F3F3F3" : "#1A1A1A"
    readonly property color textDim:    dark ? "#9C9C9C" : "#606060"
    readonly property color headerText: dark ? "#B4B4B4" : "#5E5E5E"

    // Interacción
    readonly property color hover:     dark ? "#383838" : "#E4E4E4"
    readonly property color selection: dark ? "#34506B" : "#D6E8F9"
    readonly property color accent:    dark ? "#60CDFF" : "#005FB8"
    readonly property color divider:   dark ? "#383838" : "#E7E7E7"

    readonly property string family: "Segoe UI"
}
