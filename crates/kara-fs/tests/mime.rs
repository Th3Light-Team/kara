//! Tipo MIME por nombre, con las reglas de `shared-mime-info`.

use kara_fs::mime::{glob_matches, parse_generic_icons, parse_globs2};

const BASE: &str = "\
# comentario que se ignora
50:text/plain:*.txt
50:application/gzip:*.gz
50:application/x-compressed-tar:*.tar.gz
50:text/x-csrc:*.c:cs
50:text/x-c++src:*.C:cs
50:text/x-makefile:Makefile
50:application/x-core:core:cs
50:text/x-changelog:changelog*
80:text/html:*.html
40:text/x-server-parsed-html:*.html
50:application/schema+json:*.json
50:application/json:*.json
";

#[test]
fn una_extension_corriente_se_reconoce() {
    let db = parse_globs2(BASE);
    assert_eq!(db.of("apuntes.txt"), Some("text/plain"));
}

#[test]
fn gana_la_extension_mas_larga() {
    // `.tar.gz` es un paquete, no un gzip a secas: los dos patrones encajan y
    // decide el mas especifico.
    let db = parse_globs2(BASE);
    assert_eq!(db.of("backup.tar.gz"), Some("application/x-compressed-tar"));
    assert_eq!(db.of("backup.gz"), Some("application/gzip"));
}

#[test]
fn a_igual_patron_decide_el_peso() {
    let db = parse_globs2(BASE);
    assert_eq!(db.of("index.html"), Some("text/html"));
}

#[test]
fn con_todo_empatado_gana_el_tipo_alfabeticamente_menor() {
    // `*.json` lo declaran dos tipos con el mismo peso y el mismo patron. La
    // spec no dice cual gana; sin una regla, decidiria el orden del fichero
    // generado y el icono cambiaria al reinstalar un paquete. `xdg-mime`
    // responde `application/json`, y este criterio da lo mismo.
    let db = parse_globs2(BASE);
    assert_eq!(db.of("package-lock.json"), Some("application/json"));
}

#[test]
fn la_extension_no_distingue_mayusculas_salvo_que_lo_pida() {
    let db = parse_globs2(BASE);
    assert_eq!(db.of("APUNTES.TXT"), Some("text/plain"));
}

#[test]
fn la_c_mayuscula_y_la_minuscula_son_lenguajes_distintos() {
    // El caso que obliga a respetar la bandera `cs`.
    let db = parse_globs2(BASE);
    assert_eq!(db.of("main.c"), Some("text/x-csrc"));
    assert_eq!(db.of("main.C"), Some("text/x-c++src"));
}

#[test]
fn un_nombre_completo_gana_a_cualquier_comodin() {
    let db = parse_globs2(BASE);
    assert_eq!(db.of("Makefile"), Some("text/x-makefile"));
}

#[test]
fn un_nombre_completo_sensible_a_mayusculas_no_encaja_al_reves() {
    let db = parse_globs2(BASE);
    assert_eq!(db.of("core"), Some("application/x-core"));
    assert_eq!(db.of("CORE"), None);
}

#[test]
fn un_comodin_que_no_es_de_extension_tambien_encaja() {
    let db = parse_globs2(BASE);
    assert_eq!(db.of("changelog.old"), Some("text/x-changelog"));
}

#[test]
fn un_nombre_sin_tipo_conocido_no_se_inventa() {
    let db = parse_globs2(BASE);
    assert_eq!(db.of("cosa.qwertyuiop"), None);
}

#[test]
fn el_nombre_que_es_solo_la_extension_no_cuenta_como_tal() {
    // `.txt` es un fichero oculto llamado «txt», no un texto.
    let db = parse_globs2(BASE);
    assert_eq!(db.of(".txt"), None);
}

#[test]
fn una_linea_rota_no_tumba_el_troceo() {
    let db = parse_globs2("esto no es una linea\n50:text/plain:*.txt\nno:numero:*.x\n");
    assert_eq!(db.of("a.txt"), Some("text/plain"));
    assert_eq!(db.of("a.x"), None);
}

#[test]
fn los_iconos_genericos_se_trocean() {
    let icons = parse_generic_icons("# cabecera\napplication/x-compressed-tar:package-x-generic\n");
    assert_eq!(
        icons.get("application/x-compressed-tar").map(String::as_str),
        Some("package-x-generic")
    );
}

#[test]
fn el_comodin_cubre_lo_que_dice_cubrir() {
    assert!(glob_matches("*", "cualquiera"));
    assert!(glob_matches("a*c", "abbbc"));
    assert!(glob_matches("a?c", "abc"));
    assert!(!glob_matches("a?c", "abbc"));
    assert!(glob_matches("*.out*", "callgrind.out.1234"));
    assert!(!glob_matches("a*c", "abcd"));
}

#[test]
fn un_patron_lleno_de_estrellas_no_se_dispara() {
    // Con retroceso ingenuo esto tarda una eternidad; con el iterativo, no.
    let pattern = "*a*a*a*a*a*a*a*a*a*b";
    let name = "a".repeat(64);
    assert!(!glob_matches(pattern, &name));
}

#[test]
fn la_base_del_sistema_reconoce_lo_corriente() {
    // Depende del sistema: si no esta instalada `shared-mime-info`, no hay nada
    // que comprobar y la prueba se salta en vez de fallar por el entorno.
    let db = kara_fs::mime::MimeDatabase::load();
    if db.is_empty() {
        return;
    }

    assert_eq!(db.of("foto.png"), Some("image/png"));
    assert_eq!(db.of("apuntes.md"), Some("text/markdown"));
    assert_eq!(db.of("package-lock.json"), Some("application/json"));
    assert_eq!(db.of("modulo.rs"), Some("text/rust"));
}

// --- Descripciones de tipo ------------------------------------------------

use kara_fs::mime::{MimeDescriptions, parse_comment};

const XML: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<mime-type xmlns="http://www.freedesktop.org/standards/shared-mime-info" type="application/json">
  <!--Created automatically by update-mime-database. DO NOT EDIT!-->
  <comment>JSON document</comment>
  <comment xml:lang="pt-BR">Documento JSON</comment>
  <comment xml:lang="es">documento JSON</comment>
  <glob pattern="*.json"/>
</mime-type>
"#;

fn langs(list: &[&str]) -> Vec<String> {
    list.iter().map(|l| (*l).to_string()).collect()
}

#[test]
fn se_coge_la_traduccion_pedida() {
    assert_eq!(
        parse_comment(XML, &langs(&["es"])),
        Some("documento JSON".to_string())
    );
}

#[test]
fn manda_el_orden_de_preferencia() {
    assert_eq!(
        parse_comment(XML, &langs(&["pt-BR", "es"])),
        Some("Documento JSON".to_string())
    );
}

#[test]
fn sin_traduccion_utilizable_queda_el_original() {
    assert_eq!(
        parse_comment(XML, &langs(&["fi", "eu"])),
        Some("JSON document".to_string())
    );
    assert_eq!(parse_comment(XML, &[]), Some("JSON document".to_string()));
}

#[test]
fn las_entidades_se_deshacen() {
    let xml = "<comment>Copia &amp; pega &lt;etiqueta&gt;</comment>";
    assert_eq!(
        parse_comment(xml, &[]),
        Some("Copia & pega <etiqueta>".to_string())
    );
}

#[test]
fn un_ampersand_escapado_no_se_deshace_dos_veces() {
    // `&amp;lt;` es el texto literal «&lt;», no el caracter «<».
    let xml = "<comment>&amp;lt;</comment>";
    assert_eq!(parse_comment(xml, &[]), Some("&lt;".to_string()));
}

#[test]
fn un_xml_sin_comentarios_no_describe_nada() {
    assert_eq!(parse_comment("<mime-type type=\"a/b\"/>", &[]), None);
}

#[test]
fn una_etiqueta_a_medias_no_cuelga_el_rascado() {
    assert_eq!(parse_comment("<comment>sin cerrar", &[]), None);
    assert_eq!(parse_comment("<comment", &[]), None);
}

#[test]
fn un_tipo_con_recorrido_de_rutas_no_se_consulta() {
    // El tipo compone la ruta del XML; uno con `..` saldria de la base de datos.
    let mut descriptions = MimeDescriptions::new(langs(&["es"]));
    assert_eq!(descriptions.of("../../etc/passwd"), None);
    assert_eq!(descriptions.of("a/../../b"), None);
    assert_eq!(descriptions.of("sin-barra"), None);
}

#[test]
fn el_sistema_describe_los_tipos_corrientes() {
    let mut descriptions = MimeDescriptions::new(langs(&["es"]));
    let Some(json) = descriptions.of("application/json") else {
        // Sin `shared-mime-info` instalada no hay nada que comprobar.
        return;
    };
    assert!(json.to_lowercase().contains("json"), "descripcion: {json}");
    assert!(descriptions.of("inode/directory").is_some());
}
