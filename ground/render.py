#!/usr/bin/env python3
"""Renderiza Main.qml a PNG sin abrir ventana (plataforma offscreen).

Uso:
    THEME=light VIEW=details OUT=shot.png python3 render.py

Sirve para generar capturas del spike en CI o sin escritorio.
"""
import os
import sys

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PyQt6 import sip  # noqa: E402
from PyQt6.QtCore import QTimer, QUrl  # noqa: E402
from PyQt6.QtGui import QGuiApplication  # noqa: E402
from PyQt6.QtQml import QQmlApplicationEngine  # noqa: E402
from PyQt6.QtQuick import QQuickWindow  # noqa: E402

HERE = os.path.dirname(os.path.abspath(__file__))
THEME = os.environ.get("THEME", "light")
VIEW = os.environ.get("VIEW", "details")
OUT = os.environ.get("OUT", f"shot-{THEME}-{VIEW}.png")


def main() -> int:
    app = QGuiApplication(sys.argv)
    engine = QQmlApplicationEngine()
    engine.load(QUrl.fromLocalFile(os.path.join(HERE, "Main.qml")))
    roots = engine.rootObjects()
    if not roots:
        print("ERROR: no se pudo cargar Main.qml", file=sys.stderr)
        return 1

    root = roots[0]
    root.setProperty("darkMode", THEME == "dark")
    root.setProperty("viewMode", VIEW)
    root.setProperty("effectsEnabled", False)  # sin GPU en offscreen

    # grabWindow() vive en QQuickWindow; la raíz llega como QWindow base.
    win = sip.cast(root, QQuickWindow)

    def grab() -> None:
        img = win.grabWindow()
        path = os.path.join(HERE, OUT)
        ok = img.save(path)
        print(f"{'guardado' if ok else 'FALLO'}: {path}  ({img.width()}x{img.height()})")
        app.quit()

    # Damos tiempo a que se estabilicen animaciones/fuentes antes de capturar.
    QTimer.singleShot(1400, grab)
    return app.exec()


if __name__ == "__main__":
    raise SystemExit(main())
