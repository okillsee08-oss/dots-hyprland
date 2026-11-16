//! Freedesktop desktop-entry and icon-theme lookup for workspace app icons.
//!
//! The resolver mirrors the useful part of upstream AppSearch.qml without
//! launching helpers or opening a D-Bus connection. Desktop entries are read
//! once at startup; PNG decoding is lazy and cached with strict size bounds.
//! SVG/XPM paths can be discovered, but this lightweight renderer currently
//! rasterizes PNG files only.

use std::{
    collections::HashMap,
    env, fs,
    path::{Component, Path, PathBuf},
};

use tiny_skia::Pixmap;

const MAX_DESKTOP_ENTRIES: usize = 2_048;
const MAX_DESKTOP_DEPTH: usize = 5;
const MAX_DESKTOP_FILE_BYTES: u64 = 128 * 1024;
const MAX_ICON_CACHE: usize = 128;
const MAX_ICON_FILE_BYTES: u64 = 4 * 1024 * 1024;
const MAX_ICON_DIMENSION: u32 = 512;
const MAX_ICON_THEMES: usize = 32;
const MAX_THEME_INHERITANCE_DEPTH: usize = 8;
const MAX_THEME_INDEX_BYTES: u64 = 256 * 1024;
const MAX_THEME_DIRECTORIES: usize = 64;
const MAX_ICON_SEARCH_DIRECTORIES: usize = 512;
const ICON_EXTENSIONS: [&str; 3] = ["png", "svg", "xpm"];
const ICON_SUBDIRECTORIES: [&str; 10] = [
    "scalable/apps",
    "symbolic/apps",
    "128x128/apps",
    "64x64/apps",
    "48x48/apps",
    "32x32/apps",
    "24x24/apps",
    "22x22/apps",
    "16x16/apps",
    "apps",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconResolution {
    pub name: String,
    pub path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct DesktopEntry {
    id: String,
    name: String,
    icon: String,
    startup_wm_class: Option<String>,
}

/// A process-local, bounded resolver. It performs no background work and
/// never starts one process or thread per icon.
pub struct IconResolver {
    desktop_entries: Vec<DesktopEntry>,
    icon_directories: Vec<PathBuf>,
    png_cache: HashMap<String, Option<Pixmap>>,
}

impl IconResolver {
    pub fn empty() -> Self {
        Self {
            desktop_entries: Vec::new(),
            icon_directories: Vec::new(),
            png_cache: HashMap::new(),
        }
    }

    pub fn from_environment() -> Self {
        let home = env::var_os("HOME").map(PathBuf::from);
        let data_home = env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| home.as_ref().map(|path| path.join(".local/share")));
        let data_dirs = env::var_os("XDG_DATA_DIRS")
            .map(|value| env::split_paths(&value).collect::<Vec<_>>())
            .unwrap_or_else(|| {
                vec![
                    PathBuf::from("/usr/local/share"),
                    PathBuf::from("/usr/share"),
                ]
            });

        Self::from_paths(data_home, &data_dirs, home)
    }

    /// Construct from explicit XDG roots. This is public so lookup behavior
    /// can be tested without reading or changing the host user's desktop.
    pub fn from_paths(
        data_home: Option<PathBuf>,
        data_dirs: &[PathBuf],
        home: Option<PathBuf>,
    ) -> Self {
        let mut data_roots = Vec::new();
        if let Some(root) = data_home {
            push_unique(&mut data_roots, root);
        }
        for root in data_dirs {
            push_unique(&mut data_roots, root.clone());
        }

        let mut desktop_entries = Vec::new();
        for root in &data_roots {
            scan_desktop_entries(&root.join("applications"), &mut desktop_entries);
            if desktop_entries.len() >= MAX_DESKTOP_ENTRIES {
                break;
            }
        }
        desktop_entries.sort_by(|left, right| left.id.cmp(&right.id));

        let mut themes = Vec::new();
        if let Some(theme) = home.as_deref().and_then(configured_icon_theme) {
            if !theme.is_empty() {
                push_unique(&mut themes, theme);
            }
        }
        for theme in ["hicolor", "Adwaita", "Papirus", "breeze"] {
            push_unique(&mut themes, theme.to_owned());
        }

        let mut icon_bases = Vec::new();
        for root in &data_roots {
            push_unique(&mut icon_bases, root.join("icons"));
        }
        if let Some(home) = home.as_ref() {
            push_unique(&mut icon_bases, home.join(".icons"));
        }

        let mut icon_directories = Vec::new();
        for base in &icon_bases {
            let mut visited_themes = Vec::new();
            for theme in &themes {
                collect_icon_theme_directories(
                    base,
                    theme,
                    &mut visited_themes,
                    &mut icon_directories,
                    0,
                );
            }
        }
        for root in &data_roots {
            push_unique(&mut icon_directories, root.join("pixmaps"));
        }
        if let Some(home) = home {
            push_unique(&mut icon_directories, home.join(".local/share/pixmaps"));
        }

        Self {
            desktop_entries,
            icon_directories,
            png_cache: HashMap::new(),
        }
    }

    pub fn resolve(&self, app_class: &str) -> IconResolution {
        let name = self.guess_icon_name(app_class);
        let path = self.find_icon_path(&name);
        IconResolution { name, path }
    }

    /// Return a cached PNG suitable for the small workspace marker.
    /// Missing, SVG-only, XPM-only and malformed icons are cached as misses.
    pub fn png_for_class(&mut self, app_class: &str) -> Option<&Pixmap> {
        let key = app_class.trim().to_owned();
        if key.is_empty() {
            return None;
        }

        if !self.png_cache.contains_key(&key) {
            let path = self.resolve(&key).path;
            let pixmap = path
                .filter(|path| {
                    path.extension()
                        .and_then(|extension| extension.to_str())
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("png"))
                })
                .and_then(|path| load_bounded_png(&path));

            if self.png_cache.len() >= MAX_ICON_CACHE {
                if let Some(oldest_available_key) = self.png_cache.keys().next().cloned() {
                    self.png_cache.remove(&oldest_available_key);
                }
            }
            self.png_cache.insert(key.clone(), pixmap);
        }

        self.png_cache.get(&key).and_then(Option::as_ref)
    }

    pub fn guess_icon_name(&self, app_class: &str) -> String {
        let app_class = app_class.trim();
        if app_class.is_empty() {
            return "image-missing".into();
        }

        if let Some(entry) = self
            .desktop_entry_by_id(app_class)
            .or_else(|| self.desktop_entry_by_wm_class(app_class))
        {
            return entry.icon.clone();
        }

        if let Some(replacement) = normal_substitution(app_class) {
            return replacement.into();
        }
        if let Some(replacement) = regex_substitution(app_class) {
            return replacement;
        }

        let lowercase = app_class.to_lowercase();
        let reverse_domain_name = app_class.rsplit('.').next().unwrap_or(app_class);
        let candidates = [
            app_class.to_owned(),
            lowercase.clone(),
            reverse_domain_name.to_owned(),
            reverse_domain_name.to_lowercase(),
            app_class
                .to_lowercase()
                .split_whitespace()
                .collect::<Vec<_>>()
                .join("-"),
            app_class.to_lowercase().replace('_', "-"),
        ];
        for candidate in candidates {
            if self.icon_exists(&candidate) {
                return candidate;
            }
        }

        if let Some(entry) = self.best_fuzzy_entry(app_class, true) {
            if self.icon_exists(&entry.icon) {
                return entry.icon.clone();
            }
        }
        if let Some(entry) = self.best_fuzzy_entry(app_class, false) {
            if self.icon_exists(&entry.icon) {
                return entry.icon.clone();
            }
        }
        if let Some(entry) = self.heuristic_entry(app_class) {
            return entry.icon.clone();
        }

        "application-x-executable".into()
    }

    fn desktop_entry_by_id(&self, app_class: &str) -> Option<&DesktopEntry> {
        let lowercase = app_class.to_lowercase();
        let candidates = [
            app_class.to_owned(),
            format!("{app_class}.desktop"),
            lowercase.clone(),
            format!("{lowercase}.desktop"),
        ];
        self.desktop_entries
            .iter()
            .find(|entry| candidates.contains(&entry.id))
    }

    fn desktop_entry_by_wm_class(&self, app_class: &str) -> Option<&DesktopEntry> {
        self.desktop_entries.iter().find(|entry| {
            entry
                .startup_wm_class
                .as_deref()
                .is_some_and(|value| value.eq_ignore_ascii_case(app_class))
        })
    }

    fn best_fuzzy_entry(&self, query: &str, by_icon: bool) -> Option<&DesktopEntry> {
        let mut best: Option<(&DesktopEntry, f32)> = None;
        for entry in &self.desktop_entries {
            let candidate = if by_icon {
                entry.icon.as_str()
            } else {
                entry.name.as_str()
            };
            let score = fuzzy_score(query, candidate);
            if score > 0.2 && best.is_none_or(|(_, best_score)| score > best_score) {
                best = Some((entry, score));
            }
        }
        best.map(|(entry, _)| entry)
    }

    fn heuristic_entry(&self, query: &str) -> Option<&DesktopEntry> {
        let query = normalize(query);
        self.desktop_entries.iter().find(|entry| {
            let id = entry.id.strip_suffix(".desktop").unwrap_or(&entry.id);
            normalize(id) == query
                || normalize(&entry.name) == query
                || entry
                    .startup_wm_class
                    .as_deref()
                    .is_some_and(|class| normalize(class) == query)
        })
    }

    fn icon_exists(&self, icon_name: &str) -> bool {
        self.find_icon_path(icon_name).is_some()
    }

    fn find_icon_path(&self, icon_name: &str) -> Option<PathBuf> {
        if icon_name.is_empty() {
            return None;
        }

        let supplied_path = Path::new(icon_name);
        if supplied_path.is_absolute() && supplied_path.is_file() {
            return Some(supplied_path.to_path_buf());
        }
        let file_name = supplied_path.file_name()?;
        let has_extension = supplied_path.extension().is_some();

        for directory in &self.icon_directories {
            if has_extension {
                let candidate = directory.join(file_name);
                if candidate.is_file() {
                    return Some(candidate);
                }
            } else {
                for extension in ICON_EXTENSIONS {
                    let candidate = directory.join(format!("{icon_name}.{extension}"));
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }
        }
        None
    }
}

/// Resolve the configured theme, its freedesktop inheritance chain, and the
/// bounded built-in fallbacks. Theme names and index.theme directory entries
/// are treated as untrusted path components.
fn collect_icon_theme_directories(
    base: &Path,
    theme: &str,
    visited: &mut Vec<String>,
    output: &mut Vec<PathBuf>,
    depth: usize,
) {
    if depth > MAX_THEME_INHERITANCE_DEPTH
        || visited.len() >= MAX_ICON_THEMES
        || output.len() >= MAX_ICON_SEARCH_DIRECTORIES
        || !is_safe_icon_theme_name(theme)
        || visited.iter().any(|visited_theme| visited_theme == theme)
    {
        return;
    }

    visited.push(theme.to_owned());
    let theme_root = base.join(theme);
    let index_path = theme_root.join("index.theme");
    let (inherits, mut directories) = read_icon_theme_index(&index_path).unwrap_or_default();

    if directories.is_empty() {
        directories.extend(
            ICON_SUBDIRECTORIES
                .iter()
                .map(|directory| directory.to_string()),
        );
    }

    for directory in directories.into_iter().take(MAX_THEME_DIRECTORIES) {
        if output.len() >= MAX_ICON_SEARCH_DIRECTORIES {
            break;
        }
        if is_safe_icon_theme_directory(&directory) {
            push_unique(output, theme_root.join(directory));
        }
    }

    for inherited_theme in inherits.into_iter().take(MAX_ICON_THEMES) {
        collect_icon_theme_directories(
            base,
            &inherited_theme,
            visited,
            output,
            depth.saturating_add(1),
        );
    }
}

fn read_icon_theme_index(path: &Path) -> Option<(Vec<String>, Vec<String>)> {
    if fs::metadata(path).ok()?.len() > MAX_THEME_INDEX_BYTES {
        return None;
    }
    let contents = fs::read_to_string(path).ok()?;
    let mut in_icon_theme = false;
    let mut inherits = Vec::new();
    let mut directories = Vec::new();

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            in_icon_theme = line == "[Icon Theme]";
            continue;
        }
        if !in_icon_theme {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "Inherits" => parse_icon_theme_list(value, &mut inherits),
            "Directories" | "ScaledDirectories" => parse_icon_theme_list(value, &mut directories),
            _ => {}
        }
    }

    Some((inherits, directories))
}

fn parse_icon_theme_list(value: &str, output: &mut Vec<String>) {
    for item in value
        .split(';')
        .map(str::trim)
        .filter(|item| !item.is_empty())
    {
        if item.len() <= 128 {
            push_unique(output, item.to_owned());
        }
    }
}

fn is_safe_icon_theme_name(theme: &str) -> bool {
    let mut components = Path::new(theme).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

fn is_safe_icon_theme_directory(directory: &str) -> bool {
    let path = Path::new(directory);
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn push_unique<T: PartialEq>(items: &mut Vec<T>, item: T) {
    if !items.contains(&item) {
        items.push(item);
    }
}

fn configured_icon_theme(home: &Path) -> Option<String> {
    let candidates = [
        home.join(".config/gtk-4.0/settings.ini"),
        home.join(".config/gtk-3.0/settings.ini"),
        home.join(".gtkrc-2.0"),
    ];
    for path in candidates {
        let Ok(contents) = fs::read_to_string(path) else {
            continue;
        };
        for line in contents.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            if key.trim() == "gtk-icon-theme-name" {
                let theme = value.trim().trim_matches('"').trim_matches('\'');
                if !theme.is_empty() {
                    return Some(theme.to_owned());
                }
            }
        }
    }
    None
}

fn scan_desktop_entries(applications: &Path, entries: &mut Vec<DesktopEntry>) {
    fn walk(root: &Path, directory: &Path, depth: usize, entries: &mut Vec<DesktopEntry>) {
        if depth > MAX_DESKTOP_DEPTH || entries.len() >= MAX_DESKTOP_ENTRIES {
            return;
        }
        let Ok(read_dir) = fs::read_dir(directory) else {
            return;
        };
        let mut paths = read_dir
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .collect::<Vec<_>>();
        paths.sort();

        for path in paths {
            if entries.len() >= MAX_DESKTOP_ENTRIES {
                break;
            }
            let Ok(file_type) = fs::symlink_metadata(&path).map(|metadata| metadata.file_type())
            else {
                continue;
            };
            if file_type.is_dir() {
                walk(root, &path, depth + 1, entries);
                continue;
            }
            if !file_type.is_file()
                || path.extension().and_then(|extension| extension.to_str()) != Some("desktop")
            {
                continue;
            }
            if let Some(entry) = parse_desktop_entry(root, &path) {
                entries.push(entry);
            }
        }
    }

    walk(applications, applications, 0, entries);
}

fn parse_desktop_entry(root: &Path, path: &Path) -> Option<DesktopEntry> {
    if fs::metadata(path).ok()?.len() > MAX_DESKTOP_FILE_BYTES {
        return None;
    }
    let contents = fs::read_to_string(path).ok()?;
    let relative = path.strip_prefix(root).ok()?;
    let id = relative
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "-");

    let mut in_desktop_entry = false;
    let mut name = None;
    let mut icon = None;
    let mut startup_wm_class = None;
    let mut hidden = false;

    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_desktop_entry = line == "[Desktop Entry]";
            continue;
        }
        if !in_desktop_entry || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "Name" => name = Some(value.to_owned()),
            "Icon" => icon = Some(value.to_owned()),
            "StartupWMClass" => startup_wm_class = Some(value.to_owned()),
            "Hidden" => hidden = value.eq_ignore_ascii_case("true"),
            _ => {}
        }
    }

    if hidden {
        return None;
    }
    let icon = icon.filter(|value| !value.is_empty())?;
    let name = name
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| id.strip_suffix(".desktop").unwrap_or(&id).to_owned());

    Some(DesktopEntry {
        id,
        name,
        icon,
        startup_wm_class,
    })
}

fn normal_substitution(app_class: &str) -> Option<&'static str> {
    match app_class {
        "code-url-handler" | "Code" => Some("visual-studio-code"),
        "gnome-tweaks" => Some("org.gnome.tweaks"),
        "pavucontrol-qt" => Some("pavucontrol"),
        "wps" | "wpsoffice" => Some("wps-office2019-kprometheus"),
        "footclient" => Some("foot"),
        _ => None,
    }
}

fn regex_substitution(app_class: &str) -> Option<String> {
    if let Some(id) = app_class.strip_prefix("steam_app_") {
        if !id.is_empty() && id.chars().all(|character| character.is_ascii_digit()) {
            return Some(format!("steam_icon_{id}"));
        }
    }
    if app_class.starts_with("Minecraft") {
        return Some("minecraft".into());
    }
    if app_class.contains("polkit") || app_class.contains("gcr.prompter") {
        return Some("system-lock-screen".into());
    }
    None
}

fn normalize(value: &str) -> String {
    value
        .to_lowercase()
        .replace('_', "-")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("-")
}

fn fuzzy_score(query: &str, candidate: &str) -> f32 {
    let query = normalize(query);
    let candidate = normalize(candidate);
    if query.is_empty() || candidate.is_empty() {
        return 0.0;
    }
    if candidate.contains(&query) {
        return 1.0
            - (candidate.len().saturating_sub(query.len()) as f32 / candidate.len() as f32) * 0.2;
    }

    let candidate_chars = candidate.chars().collect::<Vec<_>>();
    let query_chars = query.chars().collect::<Vec<_>>();
    if query_chars.len() > candidate_chars.len() {
        return 0.0;
    }

    let mut cursor = 0usize;
    let mut gaps = 0usize;
    for character in &query_chars {
        let Some(offset) = candidate_chars.get(cursor..).and_then(|remaining| {
            remaining
                .iter()
                .position(|candidate| candidate == character)
        }) else {
            return 0.0;
        };
        gaps += offset;
        cursor += offset + 1;
    }

    let length = candidate_chars.len() as f32;
    (query_chars.len() as f32 / length) * (1.0 - gaps as f32 / length)
}

fn load_bounded_png(path: &Path) -> Option<Pixmap> {
    if fs::metadata(path).ok()?.len() > MAX_ICON_FILE_BYTES {
        return None;
    }
    let bytes = fs::read(path).ok()?;
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" {
        return None;
    }
    let width = u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?);
    let height = u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?);
    if width == 0 || height == 0 || width > MAX_ICON_DIMENSION || height > MAX_ICON_DIMENSION {
        return None;
    }
    Pixmap::decode_png(&bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };
    use tiny_skia::Color;

    static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

    struct TestTree(PathBuf);

    impl TestTree {
        fn new() -> Self {
            let nonce = NEXT_TEMP.fetch_add(1, Ordering::Relaxed);
            let stamp = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos();
            let path = env::temp_dir().join(format!(
                "ii-shell-icon-test-{}-{stamp}-{nonce}",
                std::process::id()
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn write(&self, relative: &str, contents: &[u8]) -> PathBuf {
            let path = self.0.join(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
            path
        }
    }

    impl Drop for TestTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn png_bytes(color: Color) -> Vec<u8> {
        let mut pixmap = Pixmap::new(2, 2).unwrap();
        pixmap.fill(color);
        pixmap.encode_png().unwrap()
    }

    #[test]
    fn desktop_id_and_startup_wm_class_resolve_before_name_guesses() {
        let tree = TestTree::new();
        tree.write(
            "home/.config/gtk-3.0/settings.ini",
            b"[Settings]\ngtk-icon-theme-name=TestTheme\n",
        );
        tree.write(
            "data/applications/org.example.Editor.desktop",
            b"[Desktop Entry]\nType=Application\nName=Editor\nIcon=editor-icon\nStartupWMClass=EditorApp\n",
        );
        let icon_path = tree.write(
            "data/icons/TestTheme/48x48/apps/editor-icon.png",
            &png_bytes(Color::from_rgba8(220, 40, 50, 255)),
        );
        let data_home = tree.0.join("data");
        let home = tree.0.join("home");
        let mut resolver = IconResolver::from_paths(Some(data_home), &[], Some(home));

        let by_wm_class = resolver.resolve("EditorApp");
        assert_eq!(by_wm_class.name, "editor-icon");
        assert_eq!(by_wm_class.path.as_deref(), Some(icon_path.as_path()));
        assert_eq!(resolver.png_for_class("EditorApp").unwrap().width(), 2);

        let by_id = resolver.resolve("org.example.Editor");
        assert_eq!(by_id.name, "editor-icon");
    }

    #[test]
    fn applies_upstream_normal_and_regex_substitutions() {
        let resolver = IconResolver::empty();
        assert_eq!(resolver.guess_icon_name("Code"), "visual-studio-code");
        assert_eq!(resolver.guess_icon_name("footclient"), "foot");
        assert_eq!(
            resolver.guess_icon_name("steam_app_12345"),
            "steam_icon_12345"
        );
        assert_eq!(resolver.guess_icon_name("Minecraft 1.21"), "minecraft");
        assert_eq!(
            resolver.guess_icon_name("gcr.prompter"),
            "system-lock-screen"
        );
    }

    #[test]
    fn normalizes_space_and_underscore_names_only_when_the_icon_exists() {
        let tree = TestTree::new();
        tree.write(
            "data/icons/hicolor/48x48/apps/my-app.png",
            b"not-a-png-but-an-existing-icon-file",
        );
        tree.write(
            "data/icons/hicolor/48x48/apps/my-editor.png",
            b"not-a-png-but-an-existing-icon-file",
        );
        let resolver = IconResolver::from_paths(Some(tree.0.join("data")), &[], None);

        assert_eq!(resolver.guess_icon_name("My App"), "my-app");
        assert_eq!(resolver.guess_icon_name("My_Editor"), "my-editor");
    }

    #[test]
    fn fuzzy_desktop_icon_and_name_searches_require_a_real_icon_path() {
        let tree = TestTree::new();
        tree.write(
            "data/applications/firefox.desktop",
            b"[Desktop Entry]\nType=Application\nName=Mozilla Firefox\nIcon=firefox-icon\n",
        );
        tree.write(
            "data/icons/hicolor/48x48/apps/firefox-icon.png",
            b"icon-placeholder",
        );
        let resolver = IconResolver::from_paths(Some(tree.0.join("data")), &[], None);

        assert_eq!(resolver.guess_icon_name("firefox-ico"), "firefox-icon");
        assert_eq!(resolver.guess_icon_name("mozfire"), "firefox-icon");
    }

    #[test]
    fn svg_paths_are_discovered_but_not_misreported_as_decoded_pngs() {
        let tree = TestTree::new();
        tree.write(
            "data/applications/org.example.Vector.desktop",
            b"[Desktop Entry]\nName=Vector App\nIcon=vector-app\n",
        );
        let svg_path = tree.write(
            "data/icons/hicolor/scalable/apps/vector-app.svg",
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
        );
        let mut resolver = IconResolver::from_paths(Some(tree.0.join("data")), &[], None);

        assert_eq!(resolver.resolve("org.example.Vector").path, Some(svg_path));
        assert!(resolver.png_for_class("org.example.Vector").is_none());
    }

    #[test]
    fn follows_index_theme_directories_and_inheritance_without_recursing_forever() {
        let tree = TestTree::new();
        tree.write(
            "home/.config/gtk-4.0/settings.ini",
            b"[Settings]\ngtk-icon-theme-name=Custom\n",
        );
        tree.write(
            "data/icons/Custom/index.theme",
            b"[Icon Theme]\nDirectories=48x48/apps;\nInherits=Parent;\n[48x48/apps]\nSize=48\nType=Fixed\n",
        );
        tree.write(
            "data/icons/Parent/index.theme",
            b"[Icon Theme]\nDirectories=scalable/apps;\nInherits=Custom;\n[scalable/apps]\nSize=48\nType=Scalable\n",
        );
        let custom_icon = tree.write("data/icons/Custom/48x48/apps/theme-order.png", b"custom");
        tree.write("data/icons/Parent/scalable/apps/theme-order.png", b"parent");
        let inherited_icon = tree.write(
            "data/icons/Parent/scalable/apps/inherited-only.png",
            b"parent-only",
        );

        let resolver =
            IconResolver::from_paths(Some(tree.0.join("data")), &[], Some(tree.0.join("home")));

        assert_eq!(
            resolver.find_icon_path("theme-order").as_deref(),
            Some(custom_icon.as_path())
        );
        assert_eq!(
            resolver.find_icon_path("inherited-only").as_deref(),
            Some(inherited_icon.as_path())
        );
    }

    #[test]
    fn rejects_theme_path_traversal_and_falls_back_to_hicolor() {
        let tree = TestTree::new();
        tree.write(
            "home/.config/gtk-4.0/settings.ini",
            b"[Settings]\ngtk-icon-theme-name=Custom\n",
        );
        tree.write(
            "data/icons/Custom/index.theme",
            b"[Icon Theme]\nDirectories=../../outside/apps;\nInherits=../outside;\n",
        );
        let fallback_icon = tree.write(
            "data/icons/hicolor/48x48/apps/fallback-only.png",
            b"hicolor",
        );
        tree.write(
            "data/outside/apps/fallback-only.png",
            b"must-not-be-followed",
        );

        let resolver =
            IconResolver::from_paths(Some(tree.0.join("data")), &[], Some(tree.0.join("home")));

        assert_eq!(
            resolver.find_icon_path("fallback-only").as_deref(),
            Some(fallback_icon.as_path())
        );
        assert!(!is_safe_icon_theme_name("../outside"));
        assert!(!is_safe_icon_theme_directory("../../outside/apps"));
        assert!(is_safe_icon_theme_directory("scalable/apps"));
    }

    #[test]
    fn rejects_png_dimensions_above_the_decode_limit_before_allocating_pixels() {
        let tree = TestTree::new();
        let path = tree.0.join("oversized.png");
        let mut bytes = vec![0u8; 24];
        bytes[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        bytes[16..20].copy_from_slice(&(MAX_ICON_DIMENSION + 1).to_be_bytes());
        bytes[20..24].copy_from_slice(&2u32.to_be_bytes());
        fs::write(&path, bytes).unwrap();

        assert!(load_bounded_png(&path).is_none());
    }
}
