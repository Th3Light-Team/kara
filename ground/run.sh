#!/usr/bin/env bash
# Arranca el spike. Si tienes el binario `qml` de Qt, úsalo (QML puro);
# si no, cae a PyQt6 como lanzador.
set -euo pipefail
cd "$(dirname "$0")"

if command -v qml6 >/dev/null 2>&1; then
    exec qml6 Main.qml
elif command -v qml >/dev/null 2>&1; then
    exec qml Main.qml
else
    exec python3 main.py
fi
