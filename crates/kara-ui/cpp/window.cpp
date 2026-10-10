#include "window.h"

#include <QtCore/QByteArray>
#include <QtCore/QString>
#include <QtGui/QGuiApplication>
#include <QtGui/QWindow>

namespace kara {

void set_desktop_file_name(::rust::Str name)
{
    QGuiApplication::setDesktopFileName(
        QString::fromUtf8(name.data(), static_cast<qsizetype>(name.size())));
}

void present_window(::rust::Str activation_token)
{
    const auto windows = QGuiApplication::topLevelWindows();
    for (QWindow *window : windows) {
        // Menus and tooltips are top-level windows too; the main window is the
        // one that is visible and has no transient parent.
        if (!window->isVisible() || window->transientParent() != nullptr) {
            continue;
        }
        if (!activation_token.empty()) {
            // Qt's Wayland plugin consumes and clears this variable in
            // `requestActivate`, which is exactly the hand-over a token needs.
            qputenv("XDG_ACTIVATION_TOKEN",
                    QByteArray(activation_token.data(),
                               static_cast<qsizetype>(activation_token.size())));
        }
        if (window->windowStates() & Qt::WindowMinimized) {
            window->setWindowStates(window->windowStates() & ~Qt::WindowMinimized);
        }
        window->raise();
        window->requestActivate();
        return;
    }
}

} // namespace kara
