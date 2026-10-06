use std::path::{Component, Path};

/// `BlueMapConfigManager.formatPath`: relative to the working directory, normalized, '/'-separated, backslashes
/// escaped for HOCON. Paths on another drive (Java throws there) stay absolute.
pub fn format_path(path: &Path, cwd: &Path) -> String {
    let joined = cwd.join(path);
    let abs = normalize(&joined);
    let base = normalize(cwd);
    let parts = relativize(&base, &abs).unwrap_or(abs);
    let names: Vec<String> = parts.iter().map(component_text).collect();
    let joined = match parts.first() {
        Some(Component::RootDir) => format!("/{}", names[1..].join("/")),
        Some(Component::Prefix(_)) if matches!(parts.get(1), Some(Component::RootDir)) => {
            format!("{}/{}", names[0], names[2..].join("/"))
        }
        _ => names.join("/"),
    };
    joined.replace('\\', "\\\\")
}

fn component_text(c: &Component) -> String {
    c.as_os_str().to_string_lossy().into_owned()
}

/// Lexical `Path.normalize()`: drops `.`, folds `name/..` (leading `..` of a relative path stay).
fn normalize(path: &Path) -> Vec<Component<'_>> {
    let mut out: Vec<Component> = Vec::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir if matches!(out.last(), Some(Component::Normal(_))) => {
                out.pop();
            }
            Component::ParentDir if matches!(out.last(), Some(Component::RootDir | Component::Prefix(_))) => {}
            other => out.push(other),
        }
    }
    out
}

fn same(a: &Component, b: &Component) -> bool {
    // Windows paths compare case-insensitively (like Java's WindowsPath)
    if cfg!(windows) { component_text(a).eq_ignore_ascii_case(&component_text(b)) } else { a == b }
}

fn relativize<'a>(base: &[Component<'a>], target: &[Component<'a>]) -> Option<Vec<Component<'a>>> {
    let root_len =
        |p: &[Component]| p.iter().take_while(|c| matches!(c, Component::Prefix(_) | Component::RootDir)).count();
    let (rb, rt) = (root_len(base), root_len(target));
    if rb != rt || !base[..rb].iter().zip(&target[..rt]).all(|(a, b)| same(a, b)) {
        return None;
    }
    let common = base.iter().zip(target).take_while(|(a, b)| same(a, b)).count();
    let mut out: Vec<Component> = std::iter::repeat_n(Component::ParentDir, base.len() - common).collect();
    out.extend_from_slice(&target[common..]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::format_path;
    use std::path::Path;

    #[test]
    fn relative_to_cwd() {
        let cwd = if cfg!(windows) { Path::new("C:\\srv\\bluemap") } else { Path::new("/srv/bluemap") };
        assert_eq!(format_path(Path::new("data"), cwd), "data");
        assert_eq!(format_path(&Path::new("web").join("maps"), cwd), "web/maps");
        assert_eq!(format_path(Path::new("./world/../world"), cwd), "world");
        assert_eq!(format_path(&cwd.join("x").join("y"), cwd), "x/y");
        assert_eq!(format_path(&cwd.parent().unwrap().join("other"), cwd), "../other");
        assert_eq!(format_path(Path::new(""), cwd), "");
        if cfg!(windows) {
            assert_eq!(format_path(Path::new("D:\\worlds\\a"), cwd), "D:/worlds/a");
            assert_eq!(format_path(Path::new("c:\\SRV\\bluemap\\data"), cwd), "data");
        }
    }
}
