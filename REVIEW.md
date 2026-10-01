# Kara — MVP for review

Kara is a file explorer for Linux with the look and conveniences of the Windows 11
Explorer, built natively (Rust + Qt Quick). This is a **review build**: it is meant to be
tried by hand, and this page says what to try, what should happen, and what is
deliberately not there yet.

The interface is in **Spanish** for now (there is no translation layer yet).

## Run it

You need a Linux desktop session (X11 or Wayland). Nothing else: the AppImage carries its
own Qt.

```bash
chmod +x Kara-*-x86_64.AppImage
./Kara-*-x86_64.AppImage                 # opens your home folder
./Kara-*-x86_64.AppImage ~/some/folder   # or any folder
```

Check the download first if you like: `sha256sum -c SHA256SUMS`.

Icons, file types and thumbnails come from **your desktop** (your icon theme, the
FreeDesktop MIME database, and the thumbnail cache `~/.cache/thumbnails`), so it will look
like your system, not like a fixed theme. Settings are kept in
`~/.config/kara/settings.conf`. The trash is the standard FreeDesktop one
(`~/.local/share/Trash`): anything Kara trashes, Dolphin or Nautilus can restore, and the
other way round.

### A folder to try it on

The repository has a script that builds a folder of awkward-but-realistic files — accents,
spaces, hidden files, two folders that collide on paste, files that cannot be copied,
images for thumbnails:

```bash
scripts/kara-sample              # creates ~/kara-sample
scripts/kara-sample --big        # + a 400 MB file, to watch the progress dialog
scripts/kara-sample --many       # + a folder with 20 000 files
```

Everything below refers to it. Without the repo, any folder with a few files will do for
most steps; the ones that need the sample are marked.

## What to try

Shortcuts follow Windows; the full list is in `ground/spec/07-atajos-teclado.md`.

### Open and navigate

| Do this | Expect |
|---|---|
| Double-click a folder, or select it and press **Enter** | It opens. |
| Double-click a file, or **Enter** on it | It opens in your desktop's default application. |
| **Alt+←**, **Alt+→**, **Alt+↑** | Back, forward, up. |
| Click a segment of the path bar; **Ctrl+L** | Jump to that folder; edit the path as text. **Esc** cancels. |
| Click folders in the left panel; the arrow expands | The panel follows where you are. |
| **Ctrl+T**, **Ctrl+W**, **Ctrl+Tab**, **Ctrl+Shift+T** | New, close, switch, reopen a closed tab. |
| Open a folder with 60 files (`Documents`) | It scrolls; the status bar says "Cargando…" only while it is reading. |

### Select

**Ctrl+A**, **Ctrl+click**, **Shift+click**, **Ctrl+Shift+A** (invert), and drag in empty
space for a rubber band (hold **Ctrl** to add, **Esc** to cancel it).

### Copy, move, and the questions Kara asks

Use `Conflicts/` from the sample. Select `from/*`, **Ctrl+C**, go to `to/`, **Ctrl+V**.

| What happens | Expect |
|---|---|
| A name already exists | A dialog names the item and shows size and date of both. Options: *Omitir*, *Conservar ambos*, *Combinar* (folder onto folder), *Reemplazar*. **Esc** = *Omitir*. Focus starts on the safe option. |
| The "apply to all" box | Asked once for that kind of conflict, not once per file. |
| *Reemplazar* | The old file goes to the trash first — **Ctrl+Z** brings it back. |
| `same-name` (a file onto a folder) | Never resolves itself; the dialog warns that it is probably a mistake. |
| A big copy (`--big`) | A card at the bottom right: progress, current item, speed, time left, **Cancelar**. The window stays usable. |
| Copy `Errors/unreadable.txt` with the other files | One failure question (retry / skip / skip all / cancel); the rest is not aborted; a summary at the end lists what did not happen. |
| **Ctrl+X** / **Ctrl+V** | Moves; a cut is consumed after pasting. |
| **Ctrl+Z** / **Ctrl+Y** | Undo / redo; the button's tooltip says *what* would be undone. |

Clipboard is shared with the desktop: copy in Dolphin, paste in Kara, and the other way.

### Delete

| Do this | Expect |
|---|---|
| **Delete** | Goes to the trash. Undo with **Ctrl+Z**. |
| **Shift+Delete** (try on `Disposable/`) | A confirmation names the item, or counts them. **Focus is on Cancelar**; Enter does nothing destructive. **Esc** backs out. Only *Eliminar* deletes, for good, with progress. It is not undoable and it cannot be turned off. |
| Shift+Delete on `Errors/read-only-folder/inside.txt` | The failure question appears; nothing is lost. |
| Open the trash (left panel), restore an item, empty it | Emptying asks first, focus on the safe button. |

### See and find

| Do this | Expect |
|---|---|
| The four view buttons (or **Ctrl+Shift+3/5/6/7**), **Ctrl+wheel** | Details, list, tiles, icons; zoom is one continuous scale. Each folder remembers its own. |
| Click a column header | Sorts; click again to invert. Right-click the header to move, remove or add columns. |
| `Pictures/` in icon or tile view | Thumbnails. |
| **Ctrl+H** (or the **⋯** menu) | Shows hidden files, drawn dimmed; honours the folder's `.hidden` (`Names/`). Remembered. |
| **⋯** → *Mostrar extensiones de nombre* off | Names lose their extension — except launchers: `invoice.pdf.exe` keeps it on purpose, so it can never read as `invoice.pdf`. |
| **Ctrl+F** | Live filter by name in the current folder. **Esc** clears it. |

### Names, paths, terminal, properties

| Do this | Expect |
|---|---|
| **F2** (details view) | Renames in place; only the base name is selected, so the extension is safe. |
| **Ctrl+Shift+N** | New folder, ready to be named. |
| **Ctrl+Shift+C**, or right-click → *Copiar ruta* | Path(s) on the clipboard as text, one per line; quoted only when a shell would misread them (`Names/weird "name" $HOME.txt`). |
| Right-click → *Abrir terminal aquí* | Your terminal (`$TERMINAL`, else konsole, gnome-terminal, kitty…) in that folder. |
| **Alt+Enter** | Properties: size (exact), dates, owner, group, permissions. A folder's size is counted in the background with a live count. Read-only for now. |

## Not in this build

On purpose, so nobody reports them as bugs:

- **Recursive search.** Only the live filter of the current folder (**Ctrl+F**).
- Drag and drop (between folders, tabs or panels), compress / extract, mount / eject,
  bulk rename, preview panel, "open with…", configurable shortcuts, a settings screen.
- Changing permissions (Properties is read-only), symlink creation, network locations.
- English or any other language in the UI.

## Known limits

- A folder with ~100 000 entries reads off the UI thread, but preparing it for display still
  holds the window for roughly a quarter of a second.
- Starting folder is read before the window exists; every later navigation is
  asynchronous. Switching tab while a folder is still loading drops that navigation.
- On a volume without a trash, **Delete** reports that it cannot trash the item instead of
  quietly deleting it; use **Shift+Delete** if you mean it.
- The automated end-to-end suite (`scripts/kara-e2e`) is intermittent on live desktops —
  the keyboard sometimes goes dead mid-run. Unit and integration tests (`cargo test`) are
  stable.

## Reporting what you find

Please include: what you did, what you expected, what happened, your desktop (KDE / GNOME /
other) and session (X11 / Wayland), and — for anything that touched files — whether the
files ended up where you expected. Anything that **loses or overwrites data without asking**
is the most valuable thing you can find.
