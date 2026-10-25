//! Read-only Ghostty color configuration. No subprocesses, config writes or extra dependencies.
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use fb_ui::term::TermPalette;
use fb_ui::theme::{hex, Mode};
use iced::Color;

#[derive(Debug, Clone)]
pub struct TerminalTheme {
    pub name: String,
    pub background: Color,
    pub foreground: Color,
    pub ansi: [Color; 16],
    pub cursor: Color,
    pub cursor_text: Color,
    pub selection_background: Color,
    pub selection_foreground: Color,
}
impl TerminalTheme {
    pub fn palette(&self, mode: Mode) -> TermPalette {
        TermPalette { mode, fg: self.foreground, bg: self.background, cursor: self.cursor, on_cursor: self.cursor_text, selection: self.selection_background, selection_fg: Some(self.selection_foreground), ansi: self.ansi }
    }
}
#[derive(Debug, Clone)]
pub struct GhosttyThemes {
    pub light: TerminalTheme,
    pub dark: TerminalTheme,
    pub skipped: Vec<String>,
}
impl GhosttyThemes {
    pub fn theme(&self, mode: Mode) -> &TerminalTheme { if mode.is_dark() { &self.dark } else { &self.light } }
}

fn unquote(s: &str) -> &str { s.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(s) }
pub fn color(s: &str) -> Option<Color> {
    let s = unquote(s.trim());
    let s = s.strip_prefix('#').unwrap_or(s);
    if !s.is_ascii() || !s.bytes().all(|b| b.is_ascii_hexdigit()) { return None; }
    let expanded;
    let s = if s.len() == 3 { expanded = s.chars().flat_map(|c| [c, c]).collect::<String>(); expanded.as_str() } else { s };
    (s.len() == 6).then(|| u32::from_str_radix(s, 16).ok().map(hex)).flatten()
}
fn lines(s: &str) -> Vec<(String, String)> {
    s.lines().filter_map(|line| {
        let line = line.trim();
        if line.starts_with('#') { return None; }
        let (k, v) = line.split_once('=')?;
        (!k.trim().is_empty()).then(|| (k.trim().to_owned(), unquote(v.trim()).to_owned()))
    }).collect()
}
fn expand(s: &str, home: &Path) -> PathBuf { s.strip_prefix("~/").map(|s| home.join(s)).unwrap_or_else(|| s.into()) }
fn regular(path: &Path) -> bool { std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() <= 1 << 20) }
fn read(path: &Path, home: &Path, seen: &mut HashSet<PathBuf>, depth: usize) -> Vec<(String, String)> {
    // Bound both cycles and arbitrarily large include graphs. Reject FIFOs/devices before opening.
    if depth > 32 || seen.len() >= 64 || !regular(path) { return vec![]; }
    let Ok(real) = path.canonicalize() else { return vec![] };
    if !seen.insert(real) { return vec![]; }
    let Ok(text) = std::fs::read_to_string(path) else { return vec![] };
    let (mut own, mut includes) = (vec![], vec![]);
    for (k, v) in lines(&text) { if k == "config-file" { includes.push(v); } else { own.push((k, v)); } }
    for inc in includes {
        let inc = unquote(inc.strip_prefix('?').unwrap_or(&inc));
        if inc.is_empty() { continue; }
        let p = expand(inc, home);
        let p = if p.is_absolute() { p } else { path.parent().unwrap_or(Path::new(".")).join(p) };
        own.extend(read(&p, home, seen, depth + 1));
    }
    own
}
fn names(value: &str, home: &Path) -> Option<(PathBuf, PathBuf)> {
    let valid = |s: &str| {
        let p = expand(s.trim(), home);
        (!s.trim().is_empty() && (p.is_absolute() || p.components().count() == 1) && s != "." && s != "..").then_some(p)
    };
    if !value.split(',').any(|s| s.trim().starts_with("light:") || s.trim().starts_with("dark:")) {
        let p = valid(value)?;
        return Some((p.clone(), p));
    }
    let (mut light, mut dark) = (None, None);
    for part in value.split(',').map(str::trim) {
        if let Some(s) = part.strip_prefix("light:") { light = valid(s); }
        else if let Some(s) = part.strip_prefix("dark:") { dark = valid(s); }
        else { return None; }
    }
    Some((light?, dark?))
}
const ANSI: [u32; 16] = [0x1d1f21, 0xcc6666, 0xb5bd68, 0xf0c674, 0x81a2be, 0xb294bb, 0x8abeb7, 0xc5c8c6, 0x666666, 0xd54e53, 0xb9ca4a, 0xe7c547, 0x7aa6da, 0xc397d8, 0x70c0b1, 0xeaeaea];
#[derive(Default, Clone)]
struct Colors { values: HashMap<String, String>, skipped: Vec<String> }
impl Colors {
    fn apply(&mut self, pairs: Vec<(String, String)>) {
        for (mut k, mut v) in pairs {
            if k == "palette" {
                let Some((i, c)) = v.split_once('=') else { continue };
                let i = i.trim();
                let index = if let Some(i) = i.strip_prefix("0x") { usize::from_str_radix(i, 16) }
                    else if let Some(i) = i.strip_prefix("0o") { usize::from_str_radix(i, 8) }
                    else if let Some(i) = i.strip_prefix("0b") { usize::from_str_radix(i, 2) }
                    else { i.parse::<usize>() };
                let Ok(i @ 0..=15) = index else { continue };
                k = format!("palette-{i}"); v = c.trim().to_owned();
            } else if !["background", "foreground", "cursor-color", "cursor-text", "selection-background", "selection-foreground"].contains(&k.as_str()) { continue; }
            if v.is_empty() || color(&v).is_some() || (!k.starts_with("palette-") && ["cell-foreground", "cell-background"].contains(&v.as_str())) {
                self.values.insert(k, v);
            } else { self.skipped.push(format!("{k} = {v}")); }
        }
    }
    fn resolve(&self, name: String) -> TerminalTheme {
        let plain = |key: &str, fallback| self.values.get(key).and_then(|s| color(s)).unwrap_or(fallback);
        let bg = plain("background", hex(0x282c34));
        let fg = plain("foreground", Color::WHITE);
        let value = |key: &str, fallback| match self.values.get(key).map(String::as_str) {
            Some("cell-foreground") => fg, Some("cell-background") => bg, _ => plain(key, fallback),
        };
        TerminalTheme { name, background: bg, foreground: fg, ansi: std::array::from_fn(|i| plain(&format!("palette-{i}"), hex(ANSI[i]))),
            cursor: value("cursor-color", fg), cursor_text: value("cursor-text", bg), selection_background: value("selection-background", fg), selection_foreground: value("selection-foreground", bg) }
    }
}
/// Injected filesystem roots keep fixtures independent of the user's config.
pub fn load(home: &Path, xdg: Option<&Path>, resources: &[PathBuf]) -> Result<GhosttyThemes, String> {
    let folder = xdg.map(Path::to_owned).unwrap_or_else(|| home.join(".config")).join("ghostty");
    let folders = [folder.clone(), home.join("Library/Application Support/com.mitchellh.ghostty")];
    let mut seen = HashSet::new();
    let mut pairs = vec![];
    let mut found = false;
    for folder in &folders { for file in ["config", "config.ghostty"] {
        let p = folder.join(file); found |= regular(&p); pairs.extend(read(&p, home, &mut seen, 0));
    } }
    if !found { return Err("No Ghostty config file found.".into()); }
    let theme = pairs.iter().filter(|(k, _)| k == "theme").next_back().map(|(_, v)| v.clone()).filter(|v| !v.is_empty());
    let mut config = Colors::default(); config.apply(pairs);
    let Some(theme) = theme else {
        if config.values.is_empty() { return Err("Ghostty's config sets no theme.".into()); }
        let t = config.resolve("Ghostty config".into());
        return Ok(GhosttyThemes { light: t.clone(), dark: t, skipped: config.skipped });
    };
    let (light, dark) = names(&theme, home).ok_or("Ghostty's theme setting is not valid.")?;
    let mut skipped = config.skipped.clone();
    let mut resolve = |name: &Path| -> Result<TerminalTheme, String> {
        let p = if name.is_absolute() { Some(name.to_owned()).filter(|p| regular(p)) } else {
            std::iter::once(folder.join("themes")).chain(resources.iter().map(|p| p.join("themes"))).map(|p| p.join(name)).find(|p| regular(p))
        }.ok_or_else(|| format!("Ghostty theme “{}” not found.", name.display()))?;
        let mut colors = Colors::default(); colors.apply(lines(&std::fs::read_to_string(p).map_err(|e| e.to_string())?));
        if colors.values.is_empty() { return Err(format!("Ghostty theme “{}” has no colors.", name.display())); }
        skipped.extend(colors.skipped.clone());
        colors.values.extend(config.values.clone());
        Ok(colors.resolve(name.file_name().unwrap_or_default().to_string_lossy().into_owned()))
    };
    let l = resolve(&light)?;
    let d = if light == dark { l.clone() } else { resolve(&dark)? };
    Ok(GhosttyThemes { light: l, dark: d, skipped })
}
pub fn load_current() -> Result<GhosttyThemes, String> {
    let home = dirs::home_dir().ok_or("Home directory unavailable.")?;
    let xdg = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()).map(PathBuf::from);
    let mut resources = vec![PathBuf::from("/usr/share/ghostty"), PathBuf::from("/usr/local/share/ghostty"), PathBuf::from("/Applications/Ghostty.app/Contents/Resources/ghostty")];
    if let Some(p) = std::env::var_os("GHOSTTY_RESOURCES_DIR") { resources.insert(0, p.into()); }
    load(&home, xdg.as_deref(), &resources)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn color_and_appearance_names_are_strict() {
        assert_eq!(color("\"#abc\""), Some(hex(0xaabbcc)));
        assert_eq!(color("red"), None);
        assert_eq!(color("ééé"), None);
        assert!(names("light:A", Path::new("/home/test")).is_none());
        assert!(names("../escape", Path::new("/home/test")).is_none());
    }
    #[test]
    fn overrides_reset_defaults_and_resolve_cell_colors() {
        let mut c = Colors::default();
        c.apply(lines("background = #123\nforeground = #456\npalette = 0xf=#abc\ncursor-color = cell-background\nselection-background = invalid"));
        let t = c.resolve("fixture".into());
        assert_eq!(t.ansi[15], hex(0xaabbcc));
        assert_eq!(t.cursor, hex(0x112233));
        assert_eq!(c.skipped.len(), 1);
        c.apply(lines("background ="));
        assert_eq!(c.resolve("reset".into()).background, hex(0x282c34));
    }
}

#[cfg(test)]
mod filesystem_tests {
    use super::*;
    #[test]
    fn includes_cycles_appearance_and_config_overrides() {
        let root = std::env::temp_dir().join(format!("flockbook-ghostty-fixture-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let dir = root.join(".config/ghostty");
        std::fs::create_dir_all(dir.join("themes")).unwrap();
        std::fs::write(dir.join("config"), "theme = light:Day,dark:Night\nforeground = #abc\nconfig-file = extra\n").unwrap();
        std::fs::write(dir.join("extra"), "background = #123\nconfig-file = config\n").unwrap();
        std::fs::write(dir.join("themes/Day"), "background = #fff\npalette = 1=#d00\n").unwrap();
        std::fs::write(dir.join("themes/Night"), "background = #000\npalette = 1=#f00\n").unwrap();
        let themes = load(&root, None, &[]).unwrap();
        assert_eq!(themes.light.background, hex(0x112233));
        assert_eq!(themes.dark.foreground, hex(0xaabbcc));
        assert_eq!(themes.light.ansi[1], hex(0xdd0000));
        assert_eq!(themes.dark.ansi[1], hex(0xff0000));
        std::fs::remove_dir_all(root).unwrap();
    }
}
