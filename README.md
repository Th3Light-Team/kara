# Kara

A file explorer for Linux with the look and conveniences of the Windows 11
Explorer, built on Rust and Qt Quick. One Kara runs natively on **GNOME** and
on **KDE Plasma**, on Wayland and on X11.

The product specification lives in [`ground/spec/`](ground/spec/); the
reviewer's guide for the current milestone is [`REVIEW.md`](REVIEW.md).

## How it fits into the desktop

Kara follows the FreeDesktop standards both desktops implement, so the same
code behaves natively on each:

| What | How |
|---|---|
| Window | Its own Fluent title bar: no server-side decorations needed (GNOME has none on Wayland) |
| Light/dark, accent colour, icon theme | Settings portal (`org.freedesktop.appearance`), followed live |
| Trash, MIME types, thumbnails, icon themes | The FreeDesktop specs, the same files Nautilus and Dolphin use |
| Cut/copy/paste of files | `text/uri-list` + `x-special/gnome-copied-files` (Nautilus) + `application/x-kde-cutselection` (Dolphin), both ways |
| Opening a file, «Abrir con» | `mimeapps.list` and `.desktop` files; `gio open`/`xdg-open`, the OpenURI portal as fallback |
| «Abrir terminal aquí» | `$TERMINAL`, then `xdg-terminal-exec`, then the known emulators (Ptyxis, GNOME Terminal, Konsole…) |
| Drives | UDisks2 over D-Bus: mount, unlock, unmount, eject, change notifications |
| Network locations | gvfs mounts (`$XDG_RUNTIME_DIR/gvfs`): SMB, SFTP, WebDAV, MTP… |
| «Show in folder» from other apps | `org.freedesktop.FileManager1` |

Everything desktop-specific lives in `crates/kara-desktop`.

## Dependencies

On Ubuntu 26.04 (GNOME or Plasma), to build and run:

```bash
sudo apt install -y build-essential cmake ninja-build pkg-config qt6-base-dev qt6-declarative-dev qt6-declarative-dev-tools qt6-svg-dev qt6-svg-plugins qt6-qpa-plugins qt6-wayland qml6-module-qtqml qml6-module-qtqml-workerscript qml6-module-qtquick qml6-module-qtquick-window qml6-module-qtquick-layouts qml6-module-qtquick-controls qml6-module-qtquick-templates qml6-module-qttest
```

`qt6-wayland` is what lets Kara run as a native Wayland client; a Plasma
install has it already, a GNOME one does not. `qml6-module-qttest` is only
needed by the end-to-end test (`scripts/kara-e2e`). The rest of the desktop
integration — `xdg-desktop-portal`, UDisks2, gvfs, `xdg-terminal-exec` — is
part of a standard Ubuntu desktop.

Rust 1.98 or newer is required.

## Build and run

```bash
cargo run -p kara-ui                  # the window
cargo run -p kara-ui -- ~/Descargas   # a folder
cargo test                            # tests
```

## Making Kara the default file manager

```bash
cargo build --release -p kara-ui
scripts/install-desktop-integration --default
```

This installs, for the current user and without root, `kara.desktop`, Kara's
icon and a D-Bus service file for `org.freedesktop.FileManager1`, and then
runs:

```bash
xdg-mime default kara.desktop inode/directory
```

From then on folders open in Kara, and «Show in folder» in Firefox, Chrome and
other applications opens Kara with the file selected: they call
`org.freedesktop.FileManager1`, directly or through the OpenURI portal, and
the session bus starts Kara for it. Kara answers on that name whenever it is
the default application for folders.

**GNOME:** Nautilus usually keeps running in the background as a D-Bus
service and holds `org.freedesktop.FileManager1` without letting another
program take it over. Kara queues for the name and gets it as soon as Nautilus
exits; to hand it over right away:

```bash
nautilus -q
```

To go back to Nautilus, `xdg-mime default org.gnome.Nautilus.desktop inode/directory`
and remove `~/.local/share/dbus-1/services/org.freedesktop.FileManager1.service`.

## AppImage

`scripts/build-appimage [version]` bundles Kara with its Qt into
`dist/Kara-<version>-x86_64.AppImage`. When the Qt it is built against has
the Wayland platform plugin and its integrations, they are bundled and the
AppImage runs natively on Wayland; otherwise it runs through XWayland. Set
`QT_QPA_PLATFORM` to override the choice.
