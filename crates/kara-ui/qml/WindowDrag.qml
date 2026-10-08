// The part of the window chrome that behaves like a title bar: dragging it
// moves the window, double-clicking it maximizes or restores.
//
// The move is the compositor's (`startSystemMove`), the only way that works on
// Wayland, and it starts once the pointer has really dragged — DragHandler's
// threshold — not on press. Handing the pointer to the compositor on press, as
// a MouseArea's `onPressed` does, lets it keep the release, and the second
// click of a double click then never reached Kara.
import QtQuick

Item {
    id: area

    required property Window window

    DragHandler {
        target: null
        onActiveChanged: {
            if (active)
                area.window.startSystemMove();
        }
    }

    TapHandler {
        // A press that turns into a drag is the DragHandler's, not a tap.
        gesturePolicy: TapHandler.DragThreshold
        onDoubleTapped: {
            if (area.window.visibility === Window.Maximized)
                area.window.showNormal();
            else
                area.window.showMaximized();
        }
    }
}
