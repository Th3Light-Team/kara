#!/usr/bin/env python3
"""Lanzador mínimo del spike QML.

QML es el motor: PyQt6 solo abre la ventana y carga Main.qml.
El binario `qml` de Qt haría exactamente lo mismo sin Python
(ver README). No hay lógica de negocio aquí a propósito.
"""
import os
import sys

from PyQt6.QtCore import QUrl
from PyQt6.QtGui import QGuiApplication
from PyQt6.QtQml import QQmlApplicationEngine

HERE = os.path.dirname(os.path.abspath(__file__))


def main() -> int:
    app = QGuiApplication(sys.argv)
    app.setApplicationName("QML File Explorer Spike")

    engine = QQmlApplicationEngine()
    engine.load(QUrl.fromLocalFile(os.path.join(HERE, "Main.qml")))
    if not engine.rootObjects():
        print("ERROR: no se pudo cargar Main.qml", file=sys.stderr)
        return 1
    return app.exec()


if __name__ == "__main__":
    raise SystemExit(main())
