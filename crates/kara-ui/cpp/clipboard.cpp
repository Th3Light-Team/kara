#include "clipboard.h"

#include <QtCore/QByteArray>
#include <QtCore/QMimeData>
#include <QtGui/QClipboard>
#include <QtGui/QGuiApplication>

namespace {

const char *const URI_LIST = "text/uri-list";
const char *const GNOME_COPIED = "x-special/gnome-copied-files";
const char *const KDE_CUT = "application/x-kde-cutselection";

/// Reads one format off the clipboard as raw bytes.
///
/// The data is returned as bytes and not as text on purpose: a path on Linux is
/// a byte string, and going through a text codec would rewrite anything that is
/// not valid UTF-8 before Rust ever sees it.
QByteArray format_data(const char *format)
{
    QClipboard *clipboard = QGuiApplication::clipboard();
    if (clipboard == nullptr) {
        return {};
    }
    const QMimeData *data = clipboard->mimeData();
    if (data == nullptr || !data->hasFormat(QString::fromUtf8(format))) {
        return {};
    }
    return data->data(QString::fromUtf8(format));
}

::rust::String to_rust(const QByteArray &bytes)
{
    return ::rust::String(bytes.constData(), static_cast<std::size_t>(bytes.size()));
}

} // namespace

namespace kara {

void clipboard_write(::rust::Str uri_list, ::rust::Str gnome, bool cut)
{
    QClipboard *clipboard = QGuiApplication::clipboard();
    if (clipboard == nullptr) {
        return;
    }

    // Owned by the clipboard once handed over; Qt deletes the previous one.
    auto *data = new QMimeData();
    data->setData(QString::fromUtf8(URI_LIST),
                  QByteArray(uri_list.data(), static_cast<qsizetype>(uri_list.size())));
    data->setData(QString::fromUtf8(GNOME_COPIED),
                  QByteArray(gnome.data(), static_cast<qsizetype>(gnome.size())));
    if (cut) {
        // KIO writes the literal byte '1' here; anything else reads as a copy.
        data->setData(QString::fromUtf8(KDE_CUT), QByteArray("1"));
    }

    clipboard->setMimeData(data);
}

void clipboard_clear()
{
    QClipboard *clipboard = QGuiApplication::clipboard();
    if (clipboard != nullptr) {
        clipboard->clear();
    }
}

::rust::String clipboard_uri_list() { return to_rust(format_data(URI_LIST)); }

::rust::String clipboard_gnome() { return to_rust(format_data(GNOME_COPIED)); }

bool clipboard_kde_cut() { return format_data(KDE_CUT) == QByteArray("1"); }

} // namespace kara
