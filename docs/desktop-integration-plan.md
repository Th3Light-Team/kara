# Desktop integration — milestone plan

One Kara, native on **GNOME** and **KDE Plasma**, Wayland and X11. This plan
tracks what the `gnome-integration` branch delivered, what still has to be
verified by hand on a live session, and the small details where most of the
remaining value is.

Conventions:

- Each milestone closes when every box is ticked **on a live session**, not
  when the code compiles. A box describes a gesture and what must happen.
- Priorities follow `ground/spec/08-prioridades.md`: **must** → **should** →
  **could**. A *should* does not start while a *must* is open.
- Spec references are to `ground/spec/`; when an item touches a convenience,
  its edge cases there are part of the item.
- Desktop-specific code goes in `crates/kara-desktop`, behind
  `DesktopIntegration`. The core crates stay desktop-agnostic.

Decisions already taken:

- **Double-click opens with the default application** through `gio open` /
  `xdg-open`, with the OpenURI portal only as a fallback. The portal shows its
  own app chooser for any type more than one application handles, until the
  same one has been picked three times (`xdg-desktop-portal` 1.21,
  `open-uri.c`), which contradicts «Abrir con» in `06-contexto-power.md`.
- Kara keeps one window with tabs: `FileManager1` requests open a tab, not a
  window.
- Kara answers `org.freedesktop.FileManager1` only when it is the default
  application for `inode/directory` or was started by the bus for it.

---

## M0 — Delivered (this branch)

Code done, unit-tested, `cargo clippy -D warnings` and `qmllint` clean. The
live checks are in M1.

- [x] Audit of KDE-specific assumptions (no KIO, Solid or `kioclient` in the
      tree; `kdeglobals` first in the icon-theme chain; Konsole first among
      terminals; Plasma's palette assumed for dark mode; no `desktopFileName`).
- [x] New crate `kara-desktop`: `DesktopIntegration` trait, portal-first,
      fallbacks isolated per module.
- [x] Title bar: vector caption glyphs, `DragHandler` + `startSystemMove`
      (move starts on drag, so double-click reaches Kara), double-click
      maximizes/restores, resize strips at the very edge, empty tab strip acts
      as title bar, window title is `<folder> — Kara`, app_id `kara`.
- [x] Appearance from the Settings portal, live: `color-scheme`,
      `accent-color`, GNOME's `icon-theme`. Accent pulled into readable
      lightness bands; Quick Controls palette derived from `Theme`.
- [x] Entry icons resolved at the size they are drawn (Yaru ships bitmaps).
- [x] Bundled fallback icons (folder, file, drive, network) in Rust and QML.
- [x] Clipboard checked against Nautilus 50.2's source; contract pinned in
      `crates/kara-fs/tests/clipboard_nautilus.rs`.
- [x] «Abrir con» from `mimeapps.list` + `.desktop` (GIO's default walk,
      aliases, inheritance, removed associations, recent picks), «Elegir
      otra aplicación…» with search and «Usar siempre», `Exec` field codes.
- [x] Terminal: `$TERMINAL`, `xdg-terminal-exec --dir`, then Ptyxis, kgx,
      GNOME Terminal, Konsole, … `Terminal=true` apps wrapped.
- [x] UDisks2: gvfs's visibility rules, mount, LUKS unlock dialog, unmount,
      eject/power-off of the whole drive, loop-mounted disk images, change
      notifications (debounced). Eject refused while a copy/move runs.
- [x] gvfs network locations in a «Red» section, disconnect via `gio mount -u`,
      live through `org.gtk.vfs.MountTracker`.
- [x] `org.freedesktop.FileManager1` (`ShowFolders`, `ShowItems`,
      `ShowItemProperties`), D-Bus activation with a hidden start, activation
      token handed to Qt.
- [x] Command line: paths and `file://` URIs; a file opens its folder with it
      selected.
- [x] Packaging: `kara.desktop` (`%U`), `org.freedesktop.FileManager1.service`,
      `scripts/install-desktop-integration [--default]`, AppImage bundles the
      Wayland platform plugin **with** its integrations or not at all.
- [x] README: dependencies, the exact `apt` line, how to become the default.

---

## M1 — Live verification on GNOME (must)

Ubuntu 26.04, GNOME 50, Wayland, native (`QT_QPA_PLATFORM` unset).

### Window

- [ ] Drag the title bar: the window moves; dragging a maximized window
      restores it under the pointer.
- [ ] Double-click the title bar and the empty tab strip: maximize, again:
      restore. Never a stray move.
- [ ] Resize from all four edges and four corners; the outermost pixel grabs.
- [ ] Minimize, maximize/restore glyph swaps, close; tooltips appear.
- [ ] No Qt fallback decoration anywhere (also after un-minimizing).
- [ ] Dash, Alt+Tab and the overview show Kara's icon and «<folder> — Kara»
      (needs `scripts/install-desktop-integration`).
- [ ] Same checks under XWayland: `QT_QPA_PLATFORM=xcb`.

### Appearance

- [ ] Settings → Appearance → Dark/Light while Kara is open: it follows within
      a second, menus and dialogs included.
- [ ] Each of GNOME's accent colours: selection, focus rings, the current-row
      bar follow; text on the accent stays readable in both schemes.
- [ ] Change the icon theme (`gsettings set org.gnome.desktop.interface
      icon-theme Adwaita`, back to `Yaru-sage`): icons swap live.
- [ ] Every zoom level on Yaru and Adwaita: no blurry icons, no blank ones.
- [ ] The title-bar theme toggle overrides the system until restart.

### Clipboard

- [ ] Cut in Kara → paste in Nautilus: the file **moves**; Kara's faded mark
      goes away.
- [ ] Copy in Kara → paste in Nautilus: a copy.
- [ ] Cut in Nautilus → paste in Kara: moves; copy → copies.
- [ ] Names with spaces, accents, `#`, `%`, and a non-UTF-8 name.
- [ ] Ten files at once, both directions.

### Opening

- [ ] Double-click a `.txt`, `.png`, `.pdf`: the default app, no chooser.
- [ ] «Abrir con» lists the same apps, same order, as Nautilus' «Open With».
- [ ] Pick a non-default app twice: it rises in the list (recent).
- [ ] «Elegir otra aplicación…» + «Usar siempre»: `gio mime <type>` reports
      the new default; Nautilus agrees.
- [ ] Mixed selection (`.png` + `.jpg`): only apps that open both.
- [ ] A file with no association: text editors offered.
- [ ] «Abrir terminal aquí» on a folder and on the background: Ptyxis opens
      in the right folder; with `TERMINAL=xterm` exported, xterm does.

### Drives

- [ ] Plug a USB stick: it appears within a second, removable icon.
- [ ] Eject: «Expulsando…» while it flushes, then «Ya puedes retirar
      «SanDisk …» con seguridad»; the stick powers off.
- [ ] Eject while a file on it is open in an editor: the «está en uso»
      message, nothing unmounted.
- [ ] Eject while a copy to it runs: refused with a clear message.
- [ ] Click an unmounted internal partition: polkit asks, it mounts, Kara
      opens it. Cancel the polkit prompt: nothing reported as an error.
- [ ] LUKS stick: wrong passphrase keeps the dialog with the error; right one
      mounts and opens; eject locks it.
- [ ] Right-click an `.iso` → «Montar»: it opens; eject detaches the loop.
- [ ] Browsing inside a volume that gets ejected: Kara goes home.
- [ ] Unplug without ejecting: the row disappears, no crash.

### Network

- [ ] Connect `smb://` and `sftp://` from Nautilus: «Red» shows «share en
      server» / «user en host» live.
- [ ] Browse them; disconnect from Kara's button; the row goes away.
- [ ] A phone over MTP shows as a phone.

### Being the file manager

- [ ] `scripts/install-desktop-integration --default`, then `nautilus -q`.
- [ ] Firefox (snap) → Downloads → «Show in folder»: Kara opens the folder
      with the file selected and comes to the front.
- [ ] Same from Chrome.
- [ ] Kara not running: the same starts it (D-Bus activation) with no extra
      home window flashing first.
- [ ] Kara running: a new tab, focused, file selected.
- [ ] `gdbus call --session --dest org.freedesktop.FileManager1 --object-path
      /org/freedesktop/FileManager1 --method
      org.freedesktop.FileManager1.ShowItemProperties "['file:///etc/hosts']" ""`:
      Properties opens.
- [ ] `xdg-open ~/Documentos`: opens in Kara.
- [ ] Revert to Nautilus as the README says: Nautilus answers again.

### AppImage

- [ ] `scripts/build-appimage` with the system Qt: bundle contains
      `platforms/libqwayland.so` and the three `wayland-*` plugin dirs.
- [ ] Runs natively on Wayland (`xprop` cannot select it); runs with
      `QT_QPA_PLATFORM=xcb`.
- [ ] An app started from it (a Qt one) does **not** inherit
      `QT_QPA_PLATFORM`.

---

## M2 — No regression on KDE Plasma (must)

Same machine or a VM with Plasma 6 on Wayland.

- [ ] Title bar, resize, double-click: identical to GNOME.
- [ ] Dark mode and accent follow Plasma's colour scheme live (via
      `xdg-desktop-portal-kde`).
- [ ] Icon theme from `kdeglobals` (Breeze), live change on restart at least.
- [ ] Cut in Kara → paste in Dolphin moves; Dolphin → Kara both ways.
- [ ] Default apps match Dolphin's «Open With».
- [ ] Konsole opens via `xdg-terminal-exec` or the list.
- [ ] Drives via UDisks2 behave as on GNOME; no gvfs → no «Red» section,
      no errors.
- [ ] `FileManager1`: with Kara as default and Dolphin not running, «Show in
      folder» from Firefox opens Kara.
- [ ] `scripts/kara-e2e` passes (under XWayland, see CLAUDE.md).

---

## M3 — Small details where the value is (should)

Ordered by value for someone moving from Nautilus or Dolphin.

### Integration that users notice within a day

- [ ] **Bookmarks interop.** Read GTK bookmarks
      (`~/.config/gtk-3.0/bookmarks`) and KDE places (`user-places.xbel`)
      into Acceso rápido; pinning in Kara writes them back. Users bring years
      of bookmarks. (`01-navegacion.md`, «Acceso rápido / Lugares con carpetas fijadas».)
- [ ] **Drag and drop with other apps** (`text/uri-list` out and in; move
      vs copy by volume and modifiers). Not implemented at all yet.
      (`06-contexto-power.md`, «Arrastrar y soltar con modificadores».)
- [ ] **Thumbnails for video and PDF** through the desktop's thumbnailers
      (`/usr/share/thumbnailers/*.thumbnailer`), writing the shared cache.
      Today only images are generated.
- [ ] **Activation tokens for launched apps.** On GNOME an app started from
      Kara may open behind it with an «is ready» notification. Request an
      `xdg_activation_v1` token from Qt and pass it (`XDG_ACTIVATION_TOKEN`,
      or the portal's `activation_token`).
- [ ] **Clipboard text.** Also offer `text/plain;charset=utf-8` with the paths,
      as Nautilus does, so a copied file pastes into a terminal or an editor.
- [ ] **Cut mark follows the clipboard.** Watch `QClipboard::dataChanged`:
      Nautilus clears the clipboard right after pasting a cut, and Kara's
      faded items should un-fade then.
- [ ] **Typing a remote location.** `smb://nas/share`, `sftp://host` in the
      address bar mount through `gio mount` and open the FUSE path.
- [ ] **Single instance.** `kara-ui ~/x` from a terminal, or a second launch,
      opens a tab in the running window instead of a second process.
- [ ] **Desktop notifications** (Notification portal) for «ya puedes
      retirar» and long operations finishing while Kara is not focused.

### Fit and finish

- [ ] **HiDPI icons.** Honour `Scale=` / `@2x` theme directories and the
      window's device pixel ratio; 200 % and fractional scaling stay sharp.
- [ ] **Window shadow and rounded corners** under Mutter (frameless windows
      get none): client-side shadow with a transparent margin, or accept the
      1 px border and document it.
- [ ] **Window menu** on right-click of the title bar and Alt+Space
      (`xdg_toplevel.show_window_menu`; needs Qt's private Wayland API).
- [ ] **Reduced motion** (`org.freedesktop.appearance reduced-motion`):
      disable the tab-strip and chevron animations.
- [ ] **High contrast** (`contrast`): stronger dividers and focus rings.
- [ ] **Text scaling** (`org.gnome.desktop.interface text-scaling-factor`)
      and the system UI font as an option next to the Fluent fonts.
- [ ] **Mount progress** in the pane: a spinner on the row while mounting or
      unlocking, not just the notice bar.
- [ ] **Accessible names** on title-bar buttons, the eject button and the
      dialogs (Orca reads them).
- [ ] **«Abrir con» on folders** hides Kara itself; app icons for snaps and
      Flatpaks (absolute paths, `hicolor` exports) verified.
- [ ] **Coldplug vs hotplug power-off**, as gvfs: an internal hot-swap bay
      offers «Apagar», not «Expulsar».

### Quality of the boundary

- [ ] Tests for `volumes::udisks` value parsing against a recorded
      `GetManagedObjects` reply (fixtures from this machine and a Plasma one).
- [ ] A fake `DesktopIntegration` for the bridge's logic that does not need Qt.
- [ ] CI: install the Wayland plugin set in the release job and assert the
      AppImage carries it; drop the `kdeglobals` stub in favour of setting the
      theme explicitly in the icon tests.

---

## M4 — Further out (could)

- [ ] Recent files (`recently-used.xbel`) as «Recientes» (`01-navegacion.md`,
      «Ubicaciones frecuentes y archivos recientes»).
- [ ] Templates (`XDG_TEMPLATES_DIR`) in «Nuevo» (`06-contexto-power.md`,
      «Nuevo (carpeta y desde plantilla)»).
- [ ] GNOME Shell search provider for file names.
- [ ] `.desktop` actions: «Nueva ventana».
- [ ] Respect GNOME's `button-layout` (buttons on the left) as an option.
- [ ] Flatpak packaging (then every desktop call is already portal-ready
      except UDisks2, which needs a static permission).
- [ ] UI translations: the window is Spanish-only today; GNOME's locale and
      the `.desktop` names already flow through.
