//! The `Exec` key: splitting it into arguments and expanding its field codes,
//! per the Desktop Entry Specification, «The Exec key».

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// One piece of an argument.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Piece {
    Text(String),
    /// A field code such as `f` or `U`, written unquoted.
    Code(char),
}

/// Why an `Exec` value cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExecError {
    #[error("unterminated quote")]
    UnterminatedQuote,
    #[error("empty command")]
    Empty,
}

/// Splits an `Exec` value (already string-unescaped) into arguments.
///
/// Arguments are separated by spaces and may be double-quoted; inside quotes
/// a backslash escapes `"`, `` ` ``, `$` and `\`. Field codes only count
/// outside quotes — `"%f"` is the two characters `%f`.
fn split(exec: &str) -> Result<Vec<Vec<Piece>>, ExecError> {
    let mut args: Vec<Vec<Piece>> = Vec::new();
    let mut current: Vec<Piece> = Vec::new();
    let mut text = String::new();
    let mut started = false;
    let mut chars = exec.chars().peekable();

    let flush_text = |text: &mut String, current: &mut Vec<Piece>| {
        if !text.is_empty() {
            current.push(Piece::Text(std::mem::take(text)));
        }
    };

    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' | '\n' => {
                if started {
                    flush_text(&mut text, &mut current);
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            '"' => {
                started = true;
                loop {
                    match chars.next() {
                        None => return Err(ExecError::UnterminatedQuote),
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(escaped @ ('"' | '`' | '$' | '\\')) => text.push(escaped),
                            Some(other) => {
                                text.push('\\');
                                text.push(other);
                            }
                            None => return Err(ExecError::UnterminatedQuote),
                        },
                        Some(other) => text.push(other),
                    }
                }
            }
            '%' => {
                started = true;
                match chars.next() {
                    Some('%') => text.push('%'),
                    Some(code) => {
                        flush_text(&mut text, &mut current);
                        current.push(Piece::Code(code));
                    }
                    None => text.push('%'),
                }
            }
            other => {
                started = true;
                text.push(other);
            }
        }
    }
    if started {
        flush_text(&mut text, &mut current);
        args.push(current);
    }
    if args.is_empty() {
        return Err(ExecError::Empty);
    }
    Ok(args)
}

/// What the field codes other than the file ones expand to.
#[derive(Debug, Clone, Default)]
pub struct Context<'a> {
    /// `%c`: the translated name.
    pub name: &'a str,
    /// `%i`: `--icon <Icon>`, dropped when there is no icon.
    pub icon: Option<&'a str>,
    /// `%k`: the `.desktop` file.
    pub desktop_file: Option<&'a Path>,
}

/// Expands an `Exec` value for `files`, returning one command line per
/// process to start.
///
/// `%F` and `%U` take every file in one process. `%f` and `%u` take one, so
/// several files mean several processes, as the spec asks. Without any file
/// code the application is started once and the files are not passed: it
/// declared it does not take any. `%u`/`%U` get `file://` URIs, `%f`/`%F`
/// plain paths.
pub fn expand(exec: &str, files: &[PathBuf], context: &Context<'_>) -> Result<Vec<Vec<OsString>>, ExecError> {
    let args = split(exec)?;
    let codes = || args.iter().flatten().filter_map(|p| match p {
        Piece::Code(c) => Some(*c),
        Piece::Text(_) => None,
    });
    let takes_many = codes().any(|c| c == 'F' || c == 'U');
    let takes_one = codes().any(|c| c == 'f' || c == 'u');

    // One invocation per file for the single-file codes; one for everything
    // otherwise (including no files at all).
    let batches: Vec<&[PathBuf]> = if takes_one && !takes_many && files.len() > 1 {
        files.chunks(1).collect()
    } else {
        vec![files]
    };

    let mut commands = Vec::with_capacity(batches.len());
    for batch in batches {
        let mut argv: Vec<OsString> = Vec::new();
        for arg in &args {
            expand_arg(arg, batch, context, &mut argv);
        }
        if argv.is_empty() {
            return Err(ExecError::Empty);
        }
        commands.push(argv);
    }
    Ok(commands)
}

fn expand_arg(arg: &[Piece], files: &[PathBuf], context: &Context<'_>, argv: &mut Vec<OsString>) {
    // Only `""` splits into an argument with no pieces: an empty argument the
    // application asked for, not something to drop.
    if arg.is_empty() {
        argv.push(OsString::new());
        return;
    }
    // A code standing alone may turn into zero or several arguments.
    if let [Piece::Code(code)] = arg {
        match code {
            'F' => {
                argv.extend(files.iter().map(|f| f.as_os_str().to_os_string()));
                return;
            }
            'U' => {
                argv.extend(files.iter().map(|f| OsString::from(kara_fs::file_uri(f))));
                return;
            }
            'f' => {
                if let Some(file) = files.first() {
                    argv.push(file.as_os_str().to_os_string());
                }
                return;
            }
            'u' => {
                if let Some(file) = files.first() {
                    argv.push(OsString::from(kara_fs::file_uri(file)));
                }
                return;
            }
            'i' => {
                if let Some(icon) = context.icon {
                    argv.push("--icon".into());
                    argv.push(icon.into());
                }
                return;
            }
            _ => {}
        }
    }

    // Embedded in other text, each code becomes text; the list codes take the
    // first file, which is the only sensible reading of `--file=%F`.
    let mut out = OsString::new();
    for piece in arg {
        match piece {
            Piece::Text(text) => out.push(text),
            Piece::Code('f' | 'F') => {
                if let Some(file) = files.first() {
                    out.push(file.as_os_str());
                }
            }
            Piece::Code('u' | 'U') => {
                if let Some(file) = files.first() {
                    out.push(kara_fs::file_uri(file));
                }
            }
            Piece::Code('c') => out.push(context.name),
            Piece::Code('k') => {
                if let Some(path) = context.desktop_file {
                    out.push(path.as_os_str());
                }
            }
            // `%i` inside a word, and the deprecated `%d %D %n %N %v %m`,
            // expand to nothing.
            Piece::Code(_) => {}
        }
    }
    if !out.is_empty() {
        argv.push(out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(exec: &str, files: &[&str]) -> Vec<Vec<String>> {
        let files: Vec<PathBuf> = files.iter().map(PathBuf::from).collect();
        let context = Context {
            name: "Editor de texto",
            icon: Some("org.gnome.TextEditor"),
            desktop_file: Some(Path::new("/usr/share/applications/te.desktop")),
        };
        expand(exec, &files, &context)
            .expect("valid Exec")
            .into_iter()
            .map(|argv| argv.into_iter().map(|a| a.to_string_lossy().into_owned()).collect())
            .collect()
    }

    #[test]
    fn a_list_code_takes_every_file_in_one_process() {
        assert_eq!(run("gimp %U", &["/a b.png", "/c.png"]), [["gimp", "file:///a%20b.png", "file:///c.png"]]);
        assert_eq!(run("eog %F", &["/a", "/b"]), [["eog", "/a", "/b"]]);
    }

    #[test]
    fn a_single_code_starts_one_process_per_file() {
        assert_eq!(run("vlc %f", &["/a", "/b"]), [vec!["vlc", "/a"], vec!["vlc", "/b"]]);
    }

    #[test]
    fn without_files_the_file_codes_vanish() {
        assert_eq!(run("gnome-text-editor %U", &[]), [["gnome-text-editor"]]);
    }

    #[test]
    fn without_a_file_code_the_files_are_not_passed() {
        assert_eq!(run("gnome-calculator", &["/a"]), [["gnome-calculator"]]);
    }

    #[test]
    fn quotes_group_and_protect_their_content() {
        assert_eq!(
            run(r#"sh -c "echo \"%f\" \$HOME" %f"#, &["/a"]),
            [["sh", "-c", "echo \"%f\" $HOME", "/a"]]
        );
    }

    #[test]
    fn percent_percent_is_a_literal_and_deprecated_codes_disappear() {
        assert_eq!(run("app --rate=100%% %d %f", &["/x"]), [["app", "--rate=100%", "/x"]]);
    }

    #[test]
    fn icon_name_and_desktop_file_codes_expand() {
        assert_eq!(
            run("app %i --title=%c --from=%k", &[]),
            [["app", "--icon", "org.gnome.TextEditor", "--title=Editor de texto", "--from=/usr/share/applications/te.desktop"]]
        );
    }

    #[test]
    fn an_unterminated_quote_is_an_error_not_a_guess() {
        let result = expand(r#"app "half"#, &[], &Context::default());
        assert_eq!(result, Err(ExecError::UnterminatedQuote));
        assert_eq!(expand("   ", &[], &Context::default()), Err(ExecError::Empty));
    }

    #[test]
    fn the_exec_line_the_installer_writes_reads_back_as_the_path() {
        // `scripts/install-desktop-integration` quotes the program like this.
        let written = r#""/opt/my apps/ka\$ra&\`x\\\\y" %U"#;
        let unescaped = crate::apps::desktop_entry::unescape(written);
        assert_eq!(run(&unescaped, &[]), [[r"/opt/my apps/ka$ra&`x\y"]]);
    }

    #[test]
    fn an_empty_quoted_argument_survives() {
        assert_eq!(run(r#"app "" %f"#, &["/a"]), [["app", "", "/a"]]);
    }
}
