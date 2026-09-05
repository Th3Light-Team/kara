pragma ComponentBehavior: Bound

// End-to-end harness: drives the live window with **real input**.
//
// `QtTest` synthesizes mouse and key events through Qt's own delivery path, so
// every check here goes through hit-testing, the delegates and the shortcut
// map — not through the bridge invokables. That distinction matters: the rubber
// band was once verified by calling the invokables and passed, while a mouse
// could not start it at all.
//
// Only instantiated when the binary is given `--e2e`; see `Main.qml`. The
// fixture it asserts against is the one `scripts/kara-e2e` builds.
//
// It ends the process with `Qt.exit`, so the exit code is the verdict.
import QtQuick
import QtQuick.Window
import QtTest

Item {
    id: e2e

    required property var win
    required property var app
    required property var fileView
    required property var address
    required property var filterField

    property int passed: 0
    property int failed: 0

    function ok(name, condition, detail) {
        if (condition) {
            e2e.passed++;
            console.log("E2E PASS  " + name);
        } else {
            e2e.failed++;
            console.log("E2E FAIL  " + name + (detail === undefined ? "" : "  -> " + detail));
        }
    }
    function info(text) {
        console.log("E2E   ..  " + text);
    }

    // ---- Geometry ----------------------------------------------------------
    // The details view is laid out from these two constants; they have to match
    // `FileDetails.qml` or every coordinate here lands one gap off.
    readonly property int leftMargin: 14
    readonly property int gap: 12
    readonly property int headerHeight: 30

    function areaOrigin() {
        return e2e.fileView.mapToItem(null, 0, 0);
    }
    function rowHeight() {
        return Math.max(32, e2e.app.icon_size + 8);
    }
    /// Centre of row `i`, in window coordinates.
    function rowPoint(i, dx) {
        const origin = e2e.areaOrigin();
        return Qt.point(origin.x + (dx === undefined ? 120 : dx), origin.y + e2e.headerHeight + i * e2e.rowHeight() + e2e.rowHeight() / 2);
    }
    /// Centre of the header cell for column `id`.
    function headerPoint(id) {
        const origin = e2e.areaOrigin();
        let x = origin.x + e2e.leftMargin + e2e.app.icon_size;
        for (let i = 0; i < e2e.app.column_count; ++i) {
            const width = e2e.app.column_widths[i] ?? 100;
            if (e2e.app.column_ids[i] === id)
                return Qt.point(x + e2e.gap + width / 2, origin.y + e2e.headerHeight / 2);
            x += e2e.gap + width;
        }
        return Qt.point(-1, -1);
    }
    /// How far a row's content reaches. To the right of this is empty space,
    /// which is the only place a rubber band may start.
    function contentWidth() {
        let width = e2e.leftMargin + e2e.app.icon_size;
        for (let i = 0; i < e2e.app.column_count; ++i)
            width += e2e.gap + (e2e.app.column_widths[i] ?? 0);
        return width;
    }
    function names() {
        const all = [];
        for (let i = 0; i < e2e.app.entry_count; ++i)
            all.push(e2e.app.entry_names[i]);
        return all;
    }
    function indexOf(name) {
        for (let i = 0; i < e2e.app.entry_count; ++i)
            if (e2e.app.entry_names[i] === name)
                return i;
        return -1;
    }

    TestCase {
        id: tc
        name: "kara-e2e"
        when: false

        property string root: ""

        /// Runs one block from a known state: back in the fixture folder, no
        /// filter, details view. A failure must not drag the following blocks
        /// somewhere else on disk — the project rule that a failure in a batch
        /// does not abort the batch applies to its own tests too.
        function block(name, body) {
            e2e.app.apply_filter("");
            e2e.app.navigate(tc.root);
            e2e.app.set_view(0, 0);
            e2e.win.requestActivate();
            tc.wait(250);
            if (e2e.app.path !== tc.root)
                console.log("E2E BROKE " + name + "  -> could not return to " + tc.root);
            try {
                body();
            } catch (error) {
                e2e.failed++;
                console.log("E2E BROKE " + name + "  -> " + error);
            }
            tc.wait(120);
        }

        function click(point, button, modifiers) {
            tc.mouseClick(e2e.win.contentItem, point.x, point.y, button === undefined ? Qt.LeftButton : button, modifiers === undefined ? Qt.NoModifier : modifiers);
            tc.wait(60);
        }
        function doubleClick(point) {
            tc.mouseClick(e2e.win.contentItem, point.x, point.y);
            tc.wait(30);
            tc.mouseDoubleClickSequence(e2e.win.contentItem, point.x, point.y);
            tc.wait(350);
        }
        /// Press, drag through `points`, release. The button has to be passed to
        /// every move: without it the moves carry no button state and a drag
        /// reads as a hover.
        function drag(x0, y0, points) {
            tc.mousePress(e2e.win.contentItem, x0, y0);
            tc.wait(40);
            for (let i = 0; i < points.length; ++i) {
                tc.mouseMove(e2e.win.contentItem, points[i].x, points[i].y, 30, Qt.LeftButton);
                tc.wait(40);
            }
            const last = points[points.length - 1];
            tc.mouseRelease(e2e.win.contentItem, last.x, last.y);
            tc.wait(200);
        }
        /// Polls until `predicate` holds, up to `timeout` ms. `tryVerify` is
        /// not used for this: it *throws* on timeout, which aborts the whole
        /// block instead of failing one check.
        function waitUntil(predicate, timeout) {
            const limit = timeout === undefined ? 3000 : timeout;
            for (let waited = 0; waited < limit; waited += 50) {
                if (predicate())
                    return true;
                tc.wait(50);
            }
            return predicate();
        }

        function key(code, modifiers) {
            // Re-claimed before every press. A window-context `Shortcut` only
            // fires while its window is active, and on a live desktop the focus
            // wanders: without this the suite goes red halfway through and the
            // failures describe the desktop, not Kara.
            if (!e2e.win.active) {
                e2e.win.requestActivate();
                tc.wait(60);
            }
            tc.keyClick(code, modifiers === undefined ? Qt.NoModifier : modifiers);
            tc.wait(120);
        }

        /// Whether keys reach the window, measured by an effect rather than a
        /// log line: F9 folds the navigation pane.
        function keyboardReaches() {
            const before = e2e.app.sidebar_visible;
            key(Qt.Key_F9);
            const changed = e2e.app.sidebar_visible !== before;
            key(Qt.Key_F9);
            return changed;
        }

        Component.onCompleted: Qt.callLater(function () {
            tc.wait(700);
            // A window-context `Shortcut` only fires while its window is active,
            // and under Wayland an application cannot activate itself: every
            // keyboard check would silently measure the desktop instead of
            // Kara. `scripts/kara-e2e` runs this under XWayland for that
            // reason; the warning below is what tells you it did not.
            for (let i = 0; i < 40 && !e2e.win.active; ++i) {
                e2e.win.requestActivate();
                tc.wait(100);
            }
            if (!e2e.win.active)
                console.log("E2E   ..  WARNING: the window is not active — shortcuts cannot fire. Run under QT_QPA_PLATFORM=xcb.");

            tc.run();
            console.log("E2E SUMMARY  passed=" + e2e.passed + " failed=" + e2e.failed);
            Qt.callLater(function () {
                Qt.exit(e2e.failed === 0 ? 0 : 1);
            });
        })

        function run() {
            const root = e2e.app.path;
            tc.root = root;
            e2e.info("folder = " + root + "  entries = " + e2e.app.entry_count);

            // ---- Listing -----------------------------------------------------
            block("listing", function () {
                e2e.ok("A1 lists the folder it was given", root.endsWith("/kara-e2e"), root);
                e2e.ok("A2 nine entries", e2e.app.entry_count === 9, "" + e2e.app.entry_count);
                // Folders first, and `Ñandú` after `Empty`: the accent has to
                // collate as an N, not sort past Z.
                e2e.ok("A3 folders first, accents collated", e2e.app.entry_names[0] === "Documents" && e2e.app.entry_names[2] === "Ñandú", e2e.names().join(","));
                const typeCell = e2e.app.entry_values[e2e.indexOf("package.json") * e2e.app.column_count + 2];
                e2e.ok("A4 the desktop's own MIME description", typeCell.indexOf("JSON") >= 0, typeCell);
                e2e.ok("A5 keys reach the window", keyboardReaches());
            });

            // ---- Sorting from the header -------------------------------------
            block("header sorting", function () {
                const size = e2e.headerPoint("size");
                e2e.ok("B0 there is a size header", size.x > 0);
                click(size);
                const ascending = e2e.names().join(",");
                e2e.ok("B1 sorts by size ascending", e2e.app.sort_column === "size" && e2e.app.sort_ascending, e2e.app.sort_column + "/" + e2e.app.sort_ascending);
                click(size);
                e2e.ok("B2 a second click inverts", e2e.app.sort_column === "size" && !e2e.app.sort_ascending, e2e.app.sort_column + "/" + e2e.app.sort_ascending);
                e2e.ok("B3 the order really changes", e2e.names().join(",") !== ascending);
                click(e2e.headerPoint("name"));
                e2e.ok("B4 a different column starts ascending again", e2e.app.sort_column === "name" && e2e.app.sort_ascending, e2e.app.sort_column + "/" + e2e.app.sort_ascending);
            });

            // ---- Selection with the mouse ------------------------------------
            block("selection", function () {
                click(e2e.rowPoint(3));
                e2e.ok("C1 a click selects one", e2e.app.selected_count === 1, "" + e2e.app.selected_count);
                click(e2e.rowPoint(5), Qt.LeftButton, Qt.ControlModifier);
                e2e.ok("C2 ctrl+click adds", e2e.app.selected_count === 2, "" + e2e.app.selected_count);
                click(e2e.rowPoint(7), Qt.LeftButton, Qt.ShiftModifier);
                // The ctrl+click on row 5 moved the anchor: the range is 5..7
                // and it replaces, which is what Explorer does.
                e2e.ok("C3 shift+click takes the range from the anchor", e2e.app.selected_count === 3, "" + e2e.app.selected_count);
                click(e2e.rowPoint(1));
                e2e.ok("C4 a plain click replaces", e2e.app.selected_count === 1, "" + e2e.app.selected_count);
            });

            // ---- Rubber band -------------------------------------------------
            block("rubber band", function () {
                const origin = e2e.areaOrigin();
                const x = origin.x + e2e.contentWidth() + 30;
                e2e.ok("D0 there is empty space to the right", x < origin.x + e2e.fileView.width - 4, "x=" + x);
                const row = e2e.rowHeight();
                const y0 = origin.y + e2e.headerHeight + 2 * row + 4;
                const y1 = origin.y + e2e.headerHeight + 4 * row + 4;
                drag(x, y0, [Qt.point(x - 60, y0 + 20), Qt.point(x - 60, y1)]);
                e2e.ok("D1 the band selects a run", e2e.app.selected_count === 3, "" + e2e.app.selected_count);

                drag(x, y0, [Qt.point(x - 60, y1)]);
                const before = e2e.app.selected_count;
                tc.mousePress(e2e.win.contentItem, x, origin.y + e2e.headerHeight + 6 * row + 4, Qt.LeftButton, Qt.ControlModifier);
                tc.mouseMove(e2e.win.contentItem, x - 60, origin.y + e2e.headerHeight + 7 * row, 30, Qt.LeftButton);
                tc.wait(40);
                tc.mouseRelease(e2e.win.contentItem, x - 60, origin.y + e2e.headerHeight + 7 * row);
                tc.wait(200);
                e2e.ok("D2 Ctrl adds to the band", e2e.app.selected_count === before + 2, e2e.app.selected_count + " after " + before);

                const kept = e2e.app.selected_count;
                tc.mousePress(e2e.win.contentItem, x, origin.y + e2e.headerHeight + row + 4);
                tc.mouseMove(e2e.win.contentItem, x - 60, origin.y + e2e.headerHeight + 3 * row, 30, Qt.LeftButton);
                tc.wait(40);
                key(Qt.Key_Escape);
                tc.mouseRelease(e2e.win.contentItem, x - 60, origin.y + e2e.headerHeight + 3 * row);
                tc.wait(200);
                e2e.ok("D3 Esc cancels and keeps what was selected", e2e.app.selected_count === kept, e2e.app.selected_count + " vs " + kept);
            });

            // ---- Rubber band on a scrolled view ------------------------------
            // Thirty entries do not fit. The content has moved under the
            // pointer, and the band has to keep pointing at what is on screen.
            block("rubber band, scrolled", function () {
                e2e.app.navigate(root + "/Documents");
                tc.wait(300);
                e2e.ok("E0 enough entries to scroll", e2e.app.entry_count === 30, "" + e2e.app.entry_count);
                const origin = e2e.areaOrigin();
                const x = origin.x + e2e.contentWidth() + 30;
                const row = e2e.rowHeight();
                // Wheel notches sent back to back get coalesced and barely move
                // anything; they need room to breathe.
                for (let i = 0; i < 12; ++i) {
                    tc.mouseWheel(e2e.win.contentItem, origin.x + 200, origin.y + 200, 0, -120);
                    tc.wait(60);
                }
                tc.wait(1200);
                // If the view moved, the top row is no longer index 0. Measured
                // from outside with a click, so the view needs no accessor that
                // would exist only for this test. The first click can be spent
                // stopping the movement.
                click(e2e.rowPoint(0));
                tc.wait(200);
                click(e2e.rowPoint(0));
                const top = e2e.app.focused_index;
                e2e.ok("E1 the wheel really scrolls", top > 3, "top row = " + top);
                const y0 = origin.y + e2e.headerHeight + 2 * row + 4;
                const y1 = origin.y + e2e.headerHeight + 4 * row + 4;
                // Two move points, not one: a single one leaves the band with
                // one position update, and if the view is still settling that
                // update lands somewhere else.
                drag(x, y0, [Qt.point(x - 60, y0 + 20), Qt.point(x - 60, y1)]);
                e2e.ok("E2 the band takes 3 rows after scrolling", e2e.app.selected_count === 3, "" + e2e.app.selected_count);
                if (e2e.app.selected_count > 0) {
                    let first = -1;
                    for (let i = 0; i < e2e.app.entry_count; ++i)
                        if ((e2e.app.entry_selected[i] ?? 0) !== 0) {
                            first = i;
                            break;
                        }
                    // The band starts two rows below the top one. The scroll
                    // does not land on an exact multiple of a row, so one row
                    // of slack is allowed.
                    e2e.ok("E3 it takes the rows on screen, not the ones above", Math.abs(first - (top + 2)) <= 1, "first selected = " + first + ", top = " + top);
                }
                e2e.ok("E4 keys still reach after sweeping", keyboardReaches());
            });

            // ---- Navigation ---------------------------------------------------
            block("navigation", function () {
                doubleClick(e2e.rowPoint(e2e.indexOf("Documents")));
                e2e.ok("F1 double click enters the folder", e2e.app.path.endsWith("/Documents"), e2e.app.path);
                e2e.ok("F2 lists all thirty", e2e.app.entry_count === 30, "" + e2e.app.entry_count);
                key(Qt.Key_Left, Qt.AltModifier);
                e2e.ok("F3 Alt+Left goes back", e2e.app.path === root, e2e.app.path);
                key(Qt.Key_Right, Qt.AltModifier);
                e2e.ok("F4 Alt+Right goes forward", e2e.app.path.endsWith("/Documents"), e2e.app.path);
                key(Qt.Key_Up, Qt.AltModifier);
                e2e.ok("F5 Alt+Up goes up", e2e.app.path === root, e2e.app.path);
            });

            // ---- Name filter ---------------------------------------------------
            block("filter", function () {
                key(Qt.Key_F, Qt.ControlModifier);
                tc.keyClick(Qt.Key_N);
                tc.keyClick(Qt.Key_O);
                tc.keyClick(Qt.Key_T);
                // The filter is debounced; a fixed wait measures it half-applied.
                waitUntil(function () {
                    return e2e.app.entry_count === 2;
                });
                e2e.ok("G1 the filter narrows to what contains 'not'", e2e.app.entry_count === 2, e2e.app.entry_count + ": " + e2e.names().join(","));
                key(Qt.Key_Escape);
                waitUntil(function () {
                    return e2e.app.entry_count === 9;
                });
                e2e.ok("G2 Esc clears the filter", e2e.app.entry_count === 9, "" + e2e.app.entry_count);
            });

            // ---- View modes -----------------------------------------------------
            block("view modes", function () {
                key(Qt.Key_5, Qt.ControlModifier | Qt.ShiftModifier);
                e2e.ok("H1 Ctrl+Shift+5 is list", e2e.app.view_mode === 1, "" + e2e.app.view_mode);
                key(Qt.Key_2, Qt.ControlModifier | Qt.ShiftModifier);
                e2e.ok("H2 Ctrl+Shift+2 is large icons", e2e.app.view_mode === 3 && e2e.app.icon_size === 128, e2e.app.view_mode + "/" + e2e.app.icon_size);
                key(Qt.Key_Minus, Qt.ControlModifier);
                e2e.ok("H3 Ctrl+- steps down the ladder", e2e.app.icon_size < 128, "" + e2e.app.icon_size);
                key(Qt.Key_6, Qt.ControlModifier | Qt.ShiftModifier);
                e2e.ok("H4 Ctrl+Shift+6 returns to details", e2e.app.view_mode === 0, "" + e2e.app.view_mode);
            });

            // ---- Tabs ------------------------------------------------------------
            block("tabs", function () {
                const opened = e2e.app.tab_count;
                key(Qt.Key_T, Qt.ControlModifier);
                e2e.ok("I1 Ctrl+T opens a tab", e2e.app.tab_count === opened + 1, "" + e2e.app.tab_count);
                doubleClick(e2e.rowPoint(e2e.indexOf("Documents")));
                e2e.ok("I2 the new tab navigates on its own", e2e.app.path.endsWith("/Documents"), e2e.app.path);
                key(Qt.Key_1, Qt.ControlModifier);
                // Switching tabs is not navigating: the first one is where it was.
                e2e.ok("I3 Ctrl+1 switches without navigating", e2e.app.path === root && e2e.app.active_tab === 0, e2e.app.path + " tab=" + e2e.app.active_tab);
                key(Qt.Key_9, Qt.ControlModifier);
                e2e.ok("I4 the other tab kept its folder", e2e.app.path.endsWith("/Documents"), e2e.app.path);
                key(Qt.Key_W, Qt.ControlModifier);
                e2e.ok("I5 Ctrl+W closes", e2e.app.tab_count === opened, "" + e2e.app.tab_count);
                key(Qt.Key_T, Qt.ControlModifier | Qt.ShiftModifier);
                e2e.ok("I6 Ctrl+Shift+T reopens", e2e.app.tab_count === opened + 1, "" + e2e.app.tab_count);
                key(Qt.Key_W, Qt.ControlModifier);
                tc.wait(100);
            });

            // ---- Rename ------------------------------------------------------------
            block("rename", function () {
                const notes = e2e.indexOf("notes.md");
                click(e2e.rowPoint(notes));
                key(Qt.Key_F2);
                e2e.ok("J1 F2 opens the editor", e2e.fileView.item.renamingIndex === notes, "" + e2e.fileView.item.renamingIndex);
                // Only the base name is selected, so typing replaces it and the
                // extension survives.
                tc.keyClick(Qt.Key_A);
                tc.keyClick(Qt.Key_P);
                tc.keyClick(Qt.Key_Return);
                tc.wait(300);
                e2e.ok("J2 renaming keeps the extension", e2e.indexOf("ap.md") >= 0, e2e.names().join(","));
                const renamed = e2e.indexOf("ap.md");
                if (renamed >= 0) {
                    click(e2e.rowPoint(renamed));
                    key(Qt.Key_F2);
                    for (const letter of [Qt.Key_N, Qt.Key_O, Qt.Key_T, Qt.Key_E, Qt.Key_S])
                        tc.keyClick(letter);
                    tc.keyClick(Qt.Key_Return);
                    tc.wait(300);
                    e2e.ok("J3 and back again", e2e.indexOf("notes.md") >= 0, e2e.names().join(","));
                }
            });

            // ---- New folder ----------------------------------------------------------
            block("new folder", function () {
                const before = e2e.app.entry_count;
                key(Qt.Key_N, Qt.ControlModifier | Qt.ShiftModifier);
                tc.wait(300);
                e2e.ok("K1 Ctrl+Shift+N creates one", e2e.app.entry_count === before + 1 && e2e.indexOf("Nueva carpeta") >= 0, e2e.names().join(","));
                key(Qt.Key_Z, Qt.ControlModifier);
                tc.wait(300);
                e2e.ok("K2 Ctrl+Z undoes it", e2e.app.entry_count === before, e2e.names().join(","));
            });

            // ---- Clipboard -------------------------------------------------------------
            block("clipboard", function () {
                click(e2e.rowPoint(e2e.indexOf("data.csv")));
                key(Qt.Key_C, Qt.ControlModifier);
                doubleClick(e2e.rowPoint(e2e.indexOf("Empty")));
                e2e.ok("L1 enters the empty folder", e2e.app.entry_count === 0, "" + e2e.app.entry_count);
                key(Qt.Key_V, Qt.ControlModifier);
                tc.wait(500);
                e2e.ok("L2 pastes the file", e2e.indexOf("data.csv") >= 0, e2e.names().join(","));
                key(Qt.Key_Z, Qt.ControlModifier);
                tc.wait(400);
                e2e.ok("L3 Ctrl+Z undoes the paste", e2e.app.entry_count === 0, e2e.names().join(","));
            });

            // ---- Address bar -------------------------------------------------------------
            block("address bar", function () {
                key(Qt.Key_L, Qt.ControlModifier);
                e2e.ok("M1 Ctrl+L starts editing", e2e.address.editing === true, "" + e2e.address.editing);
                key(Qt.Key_Escape);
                e2e.ok("M2 Esc cancels the edit", e2e.address.editing === false, "" + e2e.address.editing);
            });

            // ---- Selection keys and the trash ----------------------------------------------
            block("selection keys and trash", function () {
                click(e2e.rowPoint(3));
                key(Qt.Key_A, Qt.ControlModifier);
                e2e.ok("N1 Ctrl+A selects everything", e2e.app.selected_count === e2e.app.entry_count, e2e.app.selected_count + " of " + e2e.app.entry_count);
                key(Qt.Key_I, Qt.ControlModifier | Qt.ShiftModifier);
                e2e.ok("N2 Ctrl+Shift+I inverts", e2e.app.selected_count === 0, "" + e2e.app.selected_count);
                click(e2e.rowPoint(3));
                key(Qt.Key_Escape);
                e2e.ok("N3 Esc drops the selection", e2e.app.selected_count === 0, "" + e2e.app.selected_count);

                const before = e2e.app.entry_count;
                click(e2e.rowPoint(e2e.indexOf("binary.bin")));
                key(Qt.Key_Delete);
                tc.wait(400);
                e2e.ok("N4 Delete sends it to the trash", e2e.app.entry_count === before - 1, e2e.app.entry_count + " vs " + before);
                // Undo puts it back, so a green run leaves the desktop's trash
                // exactly as it found it.
                key(Qt.Key_Z, Qt.ControlModifier);
                tc.wait(400);
                e2e.ok("N5 Ctrl+Z restores it from the trash", e2e.indexOf("binary.bin") >= 0, e2e.names().join(","));
            });

            // ---- The trash is a place ----------------------------------------------------
            block("trash as a place", function () {
                const back = e2e.app.path;
                e2e.app.show_trash();
                tc.wait(400);
                e2e.ok("O1 the trash can be entered", e2e.app.in_trash === true);
                e2e.info("trash holds " + e2e.app.entry_count + " items");
                e2e.app.navigate(back);
                tc.wait(300);
                e2e.ok("O2 and left again", e2e.app.in_trash === false && e2e.app.path === back, e2e.app.path);
            });
        }
    }
}
