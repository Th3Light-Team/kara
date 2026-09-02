// Utilidades puras para el spike (mapeo de iconos y etiquetas de tipo).
.pragma library

function ext(name) {
    var i = name.lastIndexOf('.');
    return i < 0 ? '' : name.slice(i + 1).toLowerCase();
}

function iconFor(node) {
    if (node.type === 'folder')
        return '📁';
    switch (ext(node.name)) {
    case 'png': case 'jpg': case 'jpeg': case 'gif': case 'webp': case 'bmp':
        return '🖼️';
    case 'txt': case 'log': case 'ini': case 'cfg':
        return '📄';
    case 'pdf':
        return '📕';
    case 'doc': case 'docx':
        return '📘';
    case 'xls': case 'xlsx': case 'csv':
        return '📗';
    case 'ppt': case 'pptx':
        return '📙';
    case 'exe': case 'msi': case 'appimage':
        return '⚙️';
    case 'iso': case 'img':
        return '💿';
    case 'mp3': case 'wav': case 'flac': case 'ogg':
        return '🎵';
    case 'mp4': case 'mkv': case 'avi': case 'mov':
        return '🎬';
    case 'zip': case 'rar': case '7z': case 'gz': case 'tar':
        return '🗜️';
    case 'qml': case 'py': case 'js': case 'ts': case 'md':
    case 'json': case 'html': case 'css': case 'sh': case 'rs':
        return '📝';
    default:
        return '📄';
    }
}

function typeLabel(node) {
    if (node.type === 'folder')
        return 'Carpeta de archivos';
    var e = ext(node.name);
    var map = {
        png: 'Imagen PNG', jpg: 'Imagen JPEG', jpeg: 'Imagen JPEG',
        gif: 'Imagen GIF', webp: 'Imagen WebP', bmp: 'Mapa de bits',
        txt: 'Documento de texto', log: 'Archivo de registro',
        pdf: 'Documento PDF', doc: 'Documento Word', docx: 'Documento Word',
        xls: 'Hoja de cálculo', xlsx: 'Hoja de cálculo', csv: 'Valores CSV',
        ppt: 'Presentación', pptx: 'Presentación',
        exe: 'Aplicación', msi: 'Instalador de Windows', appimage: 'AppImage',
        iso: 'Imagen de disco', mp3: 'Audio MP3', wav: 'Audio WAV', flac: 'Audio FLAC',
        mp4: 'Vídeo MP4', mkv: 'Vídeo Matroska', avi: 'Vídeo AVI', mov: 'Vídeo QuickTime',
        zip: 'Archivo ZIP', rar: 'Archivo RAR', '7z': 'Archivo 7-Zip',
        qml: 'Origen QML', py: 'Origen Python', js: 'JavaScript', ts: 'TypeScript',
        md: 'Markdown', json: 'Archivo JSON', html: 'Documento HTML', css: 'Hoja de estilo',
        sh: 'Script de shell', rs: 'Origen Rust'
    };
    return map[e] || (e ? 'Archivo ' + e.toUpperCase() : 'Archivo');
}
