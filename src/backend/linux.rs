//! XDG desktop-entry discovery and verified, nontransactional xdg-utils changes.

use super::{Backend, Browser, CurrentDefaults};
use anyhow::{anyhow, bail, Context, Result};
use std::collections::{HashMap, HashSet};
use std::env;
use std::ffi::{c_char, c_int, CString, OsString};
use std::fs;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

pub struct SystemBackend {
    discovery: Discovery,
    runner: Box<dyn CommandRunner>,
}

impl SystemBackend {
    pub fn new() -> Result<Self> {
        Ok(Self {
            discovery: Discovery::from_environment()?,
            runner: Box::new(SystemRunner),
        })
    }

    fn query_scheme(&self, scheme: &str) -> Result<Option<String>> {
        let mime = format!("x-scheme-handler/{scheme}");
        let output = self.runner.run("xdg-mime", &["query", "default", &mime])?;
        let value = successful_output("xdg-mime query default", output)?;
        let value = value.trim();
        if value.is_empty() {
            return Ok(None);
        }
        if value.contains(['\n', '\r', '\0']) {
            bail!("xdg-mime returned an invalid {scheme} desktop-file ID");
        }
        Ok(Some(value.to_owned()))
    }
}

impl Backend for SystemBackend {
    fn browsers(&self) -> Result<Vec<Browser>> {
        self.discovery.browsers()
    }

    fn current(&self) -> Result<CurrentDefaults> {
        // Query both even if one tool invocation fails: the schemes can differ.
        let http = self.query_scheme("http");
        let https = self.query_scheme("https");
        match (http, https) {
            (Ok(http), Ok(https)) => Ok(CurrentDefaults { http, https }),
            (http, https) => bail!(
                "Cannot read URL defaults: {}",
                describe_defaults(&http, &https)
            ),
        }
    }

    fn set_default(&self, browser: &Browser, _interrupted: &dyn Fn() -> bool) -> Result<()> {
        // xdg-settings returns promptly and has no OS consent dialog to wait for,
        // so there is nothing to interrupt here.
        if !self
            .browsers()?
            .iter()
            .any(|candidate| candidate.id == browser.id)
        {
            bail!(
                "{} is no longer an available HTTP/HTTPS handler; run defbrow list again",
                browser.id
            );
        }

        // Neither the Exec field nor a shell is used. xdg-settings may change more
        // than URL associations, and may partially change them before failing.
        let set = self
            .runner
            .run("xdg-settings", &["set", "default-web-browser", &browser.id])
            .and_then(|output| successful_output("xdg-settings set", output));
        let check = self
            .runner
            .run(
                "xdg-settings",
                &["check", "default-web-browser", &browser.id],
            )
            .and_then(|output| successful_output("xdg-settings check", output))
            .and_then(|value| {
                if value.trim() == "yes" {
                    Ok(())
                } else {
                    bail!("xdg-settings check reported {:?}, not yes", value.trim())
                }
            });
        let http = self.query_scheme("http");
        let https = self.query_scheme("https");
        let matches =
            |result: &Result<Option<String>>| matches!(result, Ok(Some(id)) if id == &browser.id);
        if set.is_ok() && check.is_ok() && matches(&http) && matches(&https) {
            return Ok(());
        }

        let mut failures = Vec::new();
        if let Err(error) = set {
            failures.push(format!("set failed: {error:#}"));
        }
        if let Err(error) = check {
            failures.push(format!("check failed: {error:#}"));
        }
        if !matches(&http) || !matches(&https) {
            failures.push("URL defaults did not both match the requested handler".to_owned());
        }
        bail!(
            "Could not verify {} as the default browser: {}. Observed {}. Changes are not atomic and may already have partially applied; inspect defbrow current before retrying. Ensure xdg-utils is installed and run in your active desktop session, without sudo",
            browser.id,
            failures.join("; "),
            describe_defaults(&http, &https)
        )
    }
}

fn describe_defaults(http: &Result<Option<String>>, https: &Result<Option<String>>) -> String {
    fn describe(result: &Result<Option<String>>) -> String {
        match result {
            Ok(Some(id)) => format!("{id:?}"),
            Ok(None) => "unset".to_owned(),
            Err(error) => format!("unavailable ({error:#})"),
        }
    }
    format!("HTTP={}, HTTPS={}", describe(http), describe(https))
}

trait CommandRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<Output>;
}

struct SystemRunner;

impl CommandRunner for SystemRunner {
    fn run(&self, program: &str, args: &[&str]) -> Result<Output> {
        Command::new(program).args(args).output().with_context(|| {
            format!("Cannot run {program}; install xdg-utils and run in your active desktop session, without sudo")
        })
    }
}

fn successful_output(operation: &str, output: Output) -> Result<String> {
    if !output.status.success() {
        bail!(
            "{operation} failed ({}): {}. Check xdg-utils and your active desktop session",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout)
        .with_context(|| format!("{operation} returned non-UTF-8 output"))
}

struct Discovery {
    data_dirs: Vec<PathBuf>,
    executable_dirs: Vec<PathBuf>,
    locales: Vec<String>,
}

impl Discovery {
    fn from_environment() -> Result<Self> {
        Self::from_values(
            env::var_os("HOME"),
            env::var_os("XDG_DATA_HOME"),
            env::var_os("XDG_DATA_DIRS"),
            env::var_os("PATH"),
            ["LC_ALL", "LC_MESSAGES", "LANG"]
                .iter()
                .filter_map(|key| env::var(key).ok().filter(|value| !value.is_empty()))
                .next(),
        )
    }

    fn from_values(
        home: Option<OsString>,
        data_home: Option<OsString>,
        data_dirs: Option<OsString>,
        path: Option<OsString>,
        locale: Option<String>,
    ) -> Result<Self> {
        // XDG base paths must be absolute. Relative overrides are ignored.
        let data_home = data_home
            .map(PathBuf::from)
            .filter(|path| path.is_absolute());
        let data_home = match data_home {
            Some(path) => path,
            None => {
                let home = home.map(PathBuf::from).filter(|path| path.is_absolute())
                    .ok_or_else(|| anyhow!("HOME must be an absolute path when XDG_DATA_HOME is not set to an absolute path"))?;
                home.join(".local/share")
            }
        };
        let dirs = data_dirs
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| OsString::from("/usr/local/share:/usr/share"));
        let mut data_dirs = vec![data_home];
        data_dirs.extend(env::split_paths(&dirs).filter(|path| path.is_absolute()));
        Ok(Self {
            data_dirs,
            executable_dirs: path
                .map(|value| env::split_paths(&value).collect())
                .unwrap_or_default(),
            locales: locale.as_deref().map(locale_candidates).unwrap_or_default(),
        })
    }

    fn browsers(&self) -> Result<Vec<Browser>> {
        let mut seen = HashSet::new();
        let mut browsers = Vec::new();
        for directory in &self.data_dirs {
            let applications = directory.join("applications");
            let mut files = Vec::new();
            collect_desktop_files(&applications, &mut HashSet::new(), &mut files)?;
            files.sort();
            for path in files {
                let relative = path.strip_prefix(&applications)?;
                let Some(id) = desktop_id(relative) else {
                    continue;
                };
                // Even invalid or Hidden entries mask lower-priority entries.
                if !seen.insert(id.clone()) {
                    continue;
                }
                let bytes =
                    fs::read(&path).with_context(|| format!("Cannot read {}", path.display()))?;
                let Ok(contents) = std::str::from_utf8(&bytes) else {
                    continue;
                };
                let Some(entry) = parse_entry(contents) else {
                    continue;
                };
                let Some(name) = self.handler_name(&entry) else {
                    continue;
                };
                browsers.push(Browser {
                    id,
                    name,
                    detail: path.display().to_string(),
                });
            }
        }
        let mut name_counts = HashMap::new();
        for browser in &browsers {
            *name_counts.entry(browser.name.to_lowercase()).or_insert(0) += 1;
        }
        for browser in &mut browsers {
            if name_counts[&browser.name.to_lowercase()] > 1 {
                browser.name = format!("{} ({})", browser.name, browser.id);
            }
        }
        browsers.sort_by(|a, b| {
            a.name
                .to_lowercase()
                .cmp(&b.name.to_lowercase())
                .then(a.id.cmp(&b.id))
        });
        Ok(browsers)
    }

    fn handler_name(&self, entry: &HashMap<String, String>) -> Option<String> {
        if entry.get("Type")?.as_str() != "Application"
            || !boolean(entry, "Hidden", false).is_some_and(|hidden| !hidden)
        {
            return None;
        }
        if !boolean(entry, "DBusActivatable", false)?
            && !entry
                .get("Exec")
                .is_some_and(|value| !value.trim().is_empty())
        {
            return None;
        }
        let mimes = unescape_list(entry.get("MimeType")?)?;
        if !["x-scheme-handler/http", "x-scheme-handler/https"]
            .iter()
            .all(|mime| mimes.iter().any(|value| value == mime))
        {
            return None;
        }
        if let Some(value) = entry.get("TryExec") {
            let executable = unescape(value)?;
            if executable.is_empty() || !self.executable_available(&executable) {
                return None;
            }
        }
        // NoDisplay and OnlyShowIn/NotShowIn describe menus, not URL capability.
        let base = unescape(entry.get("Name")?)?;
        if base.trim().is_empty() {
            return None;
        }
        for locale in &self.locales {
            if let Some(value) = entry.get(&format!("Name[{locale}]")) {
                let name = unescape(value)?;
                if !name.trim().is_empty() {
                    return Some(name);
                }
            }
        }
        Some(base)
    }

    fn executable_available(&self, executable: &str) -> bool {
        fn is_executable(path: &Path) -> bool {
            unsafe extern "C" {
                fn access(path: *const c_char, mode: c_int) -> c_int;
            }
            if !fs::metadata(path).is_ok_and(|metadata| metadata.is_file()) {
                return false;
            }
            let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
                return false;
            };
            // SAFETY: access reads this live, NUL-terminated CString only for
            // the call's duration. X_OK (1 on Unix) checks real-user access
            // without launching the file.
            unsafe { access(path.as_ptr(), 1) == 0 }
        }
        let path = Path::new(executable);
        if path.is_absolute() {
            is_executable(path)
        } else if executable.contains('/') {
            false
        } else {
            self.executable_dirs
                .iter()
                .any(|directory| is_executable(&directory.join(path)))
        }
    }
}

fn collect_desktop_files(
    directory: &Path,
    visited: &mut HashSet<PathBuf>,
    files: &mut Vec<PathBuf>,
) -> Result<()> {
    let canonical = match fs::canonicalize(directory) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error).with_context(|| format!("Cannot access {}", directory.display()))
        }
    };
    // Track ancestors, not all visits: distinct symlink aliases have distinct IDs.
    if !visited.insert(canonical.clone()) {
        return Ok(());
    }
    let entries =
        fs::read_dir(directory).with_context(|| format!("Cannot list {}", directory.display()))?;
    let mut paths = entries
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()?;
    paths.sort();
    for path in paths {
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(error).with_context(|| format!("Cannot inspect {}", path.display()))
            }
        };
        if metadata.is_dir() {
            collect_desktop_files(&path, visited, files)?;
        } else if metadata.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension == "desktop")
        {
            files.push(path);
        }
    }
    visited.remove(&canonical);
    Ok(())
}

fn desktop_id(relative: &Path) -> Option<String> {
    relative
        .components()
        .map(|component| component.as_os_str().to_str())
        .collect::<Option<Vec<_>>>()
        .map(|parts| parts.join("-"))
}

fn parse_entry(contents: &str) -> Option<HashMap<String, String>> {
    let mut active = false;
    let mut found = false;
    let mut values = HashMap::new();
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            active = line == "[Desktop Entry]";
            if active && found {
                return None;
            }
            found |= active;
        } else if active {
            let (key, value) = line.split_once('=')?;
            if values
                .insert(key.trim().to_owned(), value.trim().to_owned())
                .is_some()
            {
                return None;
            }
        }
    }
    found.then_some(values)
}

fn boolean(entry: &HashMap<String, String>, key: &str, fallback: bool) -> Option<bool> {
    match entry.get(key).map(String::as_str) {
        None => Some(fallback),
        Some("true") => Some(true),
        Some("false") => Some(false),
        _ => None,
    }
}

fn unescape(value: &str) -> Option<String> {
    let mut result = String::new();
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        result.push(if character == '\\' {
            escaped(chars.next()?, false)?
        } else {
            character
        });
    }
    Some(result)
}

fn escaped(character: char, list: bool) -> Option<char> {
    match character {
        's' => Some(' '),
        'n' => Some('\n'),
        't' => Some('\t'),
        'r' => Some('\r'),
        '\\' => Some('\\'),
        ';' if list => Some(';'),
        _ => None,
    }
}

fn unescape_list(value: &str) -> Option<Vec<String>> {
    let mut values = Vec::new();
    let mut item = String::new();
    let mut chars = value.chars();
    while let Some(character) = chars.next() {
        match character {
            '\\' => item.push(escaped(chars.next()?, true)?),
            ';' => values.push(std::mem::take(&mut item)),
            character => item.push(character),
        }
    }
    if !item.is_empty() {
        values.push(item);
    }
    Some(values)
}

fn locale_candidates(locale: &str) -> Vec<String> {
    let (locale, modifier) = locale
        .split_once('@')
        .map_or((locale, None), |(locale, modifier)| {
            (locale, Some(modifier))
        });
    let locale = locale.split('.').next().unwrap_or(locale);
    if matches!(locale, "C" | "POSIX" | "") {
        return Vec::new();
    }
    let (language, territory) = locale
        .split_once('_')
        .map_or((locale, None), |(language, territory)| {
            (language, Some(territory))
        });
    let mut candidates = Vec::new();
    if territory.is_some() {
        if let Some(modifier) = modifier {
            candidates.push(format!("{locale}@{modifier}"));
        }
        candidates.push(locale.to_owned());
    }
    if let Some(modifier) = modifier {
        candidates.push(format!("{language}@{modifier}"));
    }
    candidates.push(language.to_owned());
    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::os::unix::fs::{symlink, PermissionsExt};
    use std::os::unix::process::ExitStatusExt;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Fixture(PathBuf);

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = env::temp_dir().join(format!(
                "defbrow-linux-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self, relative: &str) -> PathBuf {
            self.0.join(relative)
        }

        fn write(&self, relative: &str, contents: &str) -> PathBuf {
            let path = self.path(relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, contents).unwrap();
            path
        }

        fn desktop(&self, root: &str, id: &str, name: &str, extra: &str) -> PathBuf {
            self.write(&format!("{root}/applications/{id}"), &format!("[Desktop Entry]\nType=Application\nName={name}\nExec=/never-execute %u\nMimeType=x-scheme-handler/http;x-scheme-handler/https;\n{extra}"))
        }

        fn discovery(&self, roots: &[&str], locale: &str) -> Discovery {
            Discovery {
                data_dirs: roots.iter().map(|root| self.path(root)).collect(),
                executable_dirs: vec![self.path("bin")],
                locales: locale_candidates(locale),
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    enum Reply {
        Output(i32, &'static str, &'static str),
        Error(&'static str),
    }
    struct Expected {
        program: String,
        args: Vec<String>,
        reply: Reply,
    }
    type Script = Rc<RefCell<VecDeque<Expected>>>;
    struct FakeRunner(Script);

    impl CommandRunner for FakeRunner {
        fn run(&self, program: &str, args: &[&str]) -> Result<Output> {
            let expected = self.0.borrow_mut().pop_front().expect("unexpected command");
            assert_eq!(program, expected.program);
            assert_eq!(args, expected.args);
            match expected.reply {
                Reply::Error(message) => bail!("{message}"),
                Reply::Output(code, stdout, stderr) => Ok(Output {
                    status: std::process::ExitStatus::from_raw(code << 8),
                    stdout: stdout.as_bytes().to_vec(),
                    stderr: stderr.as_bytes().to_vec(),
                }),
            }
        }
    }

    fn expected(program: &str, args: &[&str], reply: Reply) -> Expected {
        Expected {
            program: program.into(),
            args: args.iter().map(|arg| (*arg).into()).collect(),
            reply,
        }
    }

    fn query(scheme: &str, reply: Reply) -> Expected {
        expected(
            "xdg-mime",
            &["query", "default", &format!("x-scheme-handler/{scheme}")],
            reply,
        )
    }

    fn output(value: &'static str) -> Reply {
        Reply::Output(0, value, "")
    }

    fn backend(discovery: Discovery, commands: Vec<Expected>) -> (SystemBackend, Script) {
        let script = Rc::new(RefCell::new(commands.into()));
        (
            SystemBackend {
                discovery,
                runner: Box::new(FakeRunner(script.clone())),
            },
            script,
        )
    }

    #[test]
    fn xdg_defaults_overrides_and_relative_paths() {
        let fixture = Fixture::new();
        let home = fixture.0.clone().into_os_string();
        let defaults = Discovery::from_values(Some(home.clone()), None, None, None, None).unwrap();
        assert_eq!(
            defaults.data_dirs,
            vec![
                fixture.path(".local/share"),
                PathBuf::from("/usr/local/share"),
                PathBuf::from("/usr/share")
            ]
        );
        let dirs = env::join_paths([
            fixture.path("flatpak/exports/share"),
            fixture.path("snap/desktop"),
            PathBuf::from("relative"),
        ])
        .unwrap();
        let overrides = Discovery::from_values(
            None,
            Some(fixture.path("custom").into_os_string()),
            Some(dirs),
            None,
            None,
        )
        .unwrap();
        assert_eq!(
            overrides.data_dirs,
            vec![
                fixture.path("custom"),
                fixture.path("flatpak/exports/share"),
                fixture.path("snap/desktop")
            ]
        );
        let relative = Discovery::from_values(
            Some(home),
            Some("relative".into()),
            Some("".into()),
            None,
            None,
        )
        .unwrap();
        assert_eq!(relative.data_dirs, defaults.data_dirs);
        assert!(Discovery::from_values(None, Some("relative".into()), None, None, None).is_err());
    }

    #[test]
    fn generic_handlers_including_exported_and_unfamiliar_apps_are_sorted() {
        let fixture = Fixture::new();
        for (id, name) in [
            ("chromium.desktop", "Chromium"),
            ("brave.desktop", "Brave"),
            ("firefox.desktop", "Firefox"),
            ("librewolf.desktop", "LibreWolf"),
            ("odd.desktop", "Unfamiliar Web Navigator"),
        ] {
            fixture.desktop("user", id, name, "");
        }
        fixture.desktop(
            "flatpak/exports/share",
            "org.example.Navigator.desktop",
            "Flatpak Navigator",
            "NoDisplay=true\nOnlyShowIn=OtherDesktop;\n",
        );
        fixture.desktop(
            "snap/desktop",
            "snap-handler.desktop",
            "Snap Handler",
            "NotShowIn=CurrentDesktop;\n",
        );
        let discovery = fixture.discovery(
            &["user", "flatpak/exports/share", "snap/desktop", "missing"],
            "C",
        );
        let browsers = discovery.browsers().unwrap();
        assert_eq!(
            browsers
                .iter()
                .map(|browser| browser.name.as_str())
                .collect::<Vec<_>>(),
            [
                "Brave",
                "Chromium",
                "Firefox",
                "Flatpak Navigator",
                "LibreWolf",
                "Snap Handler",
                "Unfamiliar Web Navigator"
            ]
        );
        assert_eq!(discovery.browsers().unwrap(), browsers);
    }

    #[test]
    fn per_id_precedence_tombstones_and_nested_ids() {
        let fixture = Fixture::new();
        fixture.desktop("system", "same.desktop", "System", "");
        fixture.desktop("user", "same.desktop", "User", "");
        fixture.desktop("system", "hidden.desktop", "Hidden system", "");
        fixture.write(
            "user/applications/hidden.desktop",
            "[Desktop Entry]\nHidden=true\n",
        );
        fixture.desktop("system", "bad.desktop", "Lower valid", "");
        fixture.write(
            "user/applications/bad.desktop",
            "invalid high-priority entry",
        );
        fixture.desktop("system", "vendor-tools-browser.desktop", "Lower nested", "");
        fixture.desktop("user", "vendor/tools/browser.desktop", "Nested", "");
        let browsers = fixture
            .discovery(&["user", "system"], "C")
            .browsers()
            .unwrap();
        assert_eq!(
            browsers
                .iter()
                .map(|browser| browser.id.as_str())
                .collect::<Vec<_>>(),
            ["vendor-tools-browser.desktop", "same.desktop"]
        );
        assert_eq!(browsers[0].name, "Nested");
        assert_eq!(browsers[1].name, "User");
    }

    #[test]
    fn duplicate_labels_disambiguate_by_id_stably() {
        let fixture = Fixture::new();
        fixture.desktop("data", "second.desktop", "Navigator", "");
        fixture.desktop("data", "first.desktop", "Navigator", "");
        let browsers = fixture.discovery(&["data"], "C").browsers().unwrap();
        assert_eq!(browsers[0].name, "Navigator (first.desktop)");
        assert_eq!(browsers[1].name, "Navigator (second.desktop)");
        assert!(browsers[0]
            .detail
            .ends_with("data/applications/first.desktop"));
    }

    #[test]
    fn localized_names_follow_country_modifier_fallback_and_string_escapes() {
        assert_eq!(
            locale_candidates("sr_RS.UTF-8@latin"),
            ["sr_RS@latin", "sr_RS", "sr@latin", "sr"]
        );
        assert_eq!(locale_candidates("de.UTF-8"), ["de"]);
        assert!(locale_candidates("C.UTF-8").is_empty());
        assert!(locale_candidates("POSIX").is_empty());
        let fixture = Fixture::new();
        fixture.desktop("data", "translated.desktop", "Base\\sName\\\\End", "Name[sr]=Language\nName[sr@latin]=Modifier\nName[sr_RS]=Country\\sName\\tTab\\nLine\\rReturn\n[Desktop Action Private]\nName=Wrong group\n");
        let discovery = fixture.discovery(&["data"], "sr_RS.UTF-8@latin");
        assert_eq!(
            discovery.browsers().unwrap()[0].name,
            "Country Name\tTab\nLine\rReturn"
        );
        assert_eq!(
            fixture
                .discovery(&["data"], "sr_ME@latin")
                .browsers()
                .unwrap()[0]
                .name,
            "Modifier"
        );
        assert_eq!(
            fixture.discovery(&["data"], "C").browsers().unwrap()[0].name,
            "Base Name\\End"
        );
        assert_eq!(
            unescape_list("one\\;part;two;"),
            Some(vec!["one;part".into(), "two".into()])
        );
        assert!(unescape("bad\\q").is_none());
        assert!(unescape("bad\\").is_none());
    }

    #[test]
    fn locale_environment_value_is_explicit_and_encoding_is_removed() {
        let fixture = Fixture::new();
        let discovery = Discovery::from_values(
            Some(fixture.0.clone().into_os_string()),
            None,
            None,
            None,
            Some("fr_CA.UTF-8".into()),
        )
        .unwrap();
        assert_eq!(discovery.locales, ["fr_CA", "fr"]);
    }

    #[test]
    fn invalid_or_single_scheme_entries_do_not_qualify() {
        let fixture = Fixture::new();
        for (id, text) in [
            ("single", "Type=Application\nName=Single\nExec=no\nMimeType=x-scheme-handler/http;"),
            ("substring", "Type=Application\nName=Wrong MIME\nExec=no\nMimeType=x-scheme-handler/http-extra;x-scheme-handler/https;"),
            ("link", "Type=Link\nName=Link\nExec=no\nMimeType=x-scheme-handler/http;x-scheme-handler/https;"),
            ("missing-exec", "Type=Application\nName=No exec\nMimeType=x-scheme-handler/http;x-scheme-handler/https;"),
            ("duplicate-key", "Type=Application\nType=Application\nName=Duplicate\nExec=no\nMimeType=x-scheme-handler/http;x-scheme-handler/https;"),
            ("empty-name", "Type=Application\nName=\nExec=no\nMimeType=x-scheme-handler/http;x-scheme-handler/https;"),
            ("bad-bool", "Type=Application\nName=Bad bool\nExec=no\nHidden=maybe\nMimeType=x-scheme-handler/http;x-scheme-handler/https;"),
        ] {
            fixture.write(&format!("data/applications/{id}.desktop"), &format!("[Desktop Entry]\n{text}\n"));
        }
        fixture.write("data/applications/dbus.desktop", "[Desktop Entry]\nType=Application\nName=Bus handler\nDBusActivatable=true\nMimeType=x-scheme-handler/http;x-scheme-handler/https;\n");
        assert_eq!(
            fixture
                .discovery(&["data"], "C")
                .browsers()
                .unwrap()
                .iter()
                .map(|browser| browser.id.as_str())
                .collect::<Vec<_>>(),
            ["dbus.desktop"]
        );
    }

    #[test]
    fn tryexec_checks_access_without_executing_and_accepts_absolute_paths() {
        let fixture = Fixture::new();
        let executable = fixture.write("bin/available", "not executable code; must never launch");
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let blocked = fixture.write("bin/blocked", "not executable");
        fs::set_permissions(&blocked, fs::Permissions::from_mode(0o600)).unwrap();
        let inaccessible = fixture.write("bin/inaccessible", "not accessible");
        fs::set_permissions(&inaccessible, fs::Permissions::from_mode(0o000)).unwrap();
        fixture.desktop(
            "data",
            "available.desktop",
            "Available",
            "TryExec=available\n",
        );
        fixture.desktop(
            "data",
            "absolute.desktop",
            "Absolute",
            &format!("TryExec={}\n", executable.display()),
        );
        fixture.desktop("data", "blocked.desktop", "Blocked", "TryExec=blocked\n");
        fixture.desktop(
            "data",
            "inaccessible.desktop",
            "Inaccessible",
            "TryExec=inaccessible\n",
        );
        fixture.desktop("data", "missing.desktop", "Missing", "TryExec=not-found\n");
        fixture.desktop(
            "data",
            "directory.desktop",
            "Directory",
            &format!("TryExec={}\n", fixture.path("bin").display()),
        );
        fixture.desktop(
            "data",
            "relative.desktop",
            "Relative",
            "TryExec=bin/available\n",
        );
        let discovery = fixture.discovery(&["data"], "C");
        assert_eq!(
            discovery
                .browsers()
                .unwrap()
                .iter()
                .map(|browser| browser.id.as_str())
                .collect::<Vec<_>>(),
            ["absolute.desktop", "available.desktop"]
        );
        assert!(!discovery.executable_available("available\0ignored"));
        assert!(!discovery.executable_available(&format!("{}\0ignored", executable.display())));
    }

    #[test]
    fn recursive_symlink_cycles_are_bounded_and_aliases_keep_ids() {
        let fixture = Fixture::new();
        fixture.desktop("data", "real/browser.desktop", "Navigator", "");
        symlink(
            fixture.path("data/applications"),
            fixture.path("data/applications/real/cycle"),
        )
        .unwrap();
        symlink(
            fixture.path("data/applications/real"),
            fixture.path("data/applications/alias"),
        )
        .unwrap();
        let browsers = fixture.discovery(&["data"], "C").browsers().unwrap();
        assert_eq!(
            browsers
                .iter()
                .map(|browser| browser.id.as_str())
                .collect::<Vec<_>>(),
            ["alias-browser.desktop", "real-browser.desktop"]
        );
    }

    #[test]
    fn current_reports_both_schemes_and_empty_default() {
        let fixture = Fixture::new();
        let (backend, script) = backend(
            fixture.discovery(&[], "C"),
            vec![
                query("http", output("firefox.desktop\n")),
                query("https", output("\n")),
            ],
        );
        assert_eq!(
            backend.current().unwrap(),
            CurrentDefaults {
                http: Some("firefox.desktop".into()),
                https: None
            }
        );
        assert!(script.borrow().is_empty());
    }

    #[test]
    fn current_failure_still_queries_the_other_scheme() {
        let fixture = Fixture::new();
        let (backend, script) = backend(
            fixture.discovery(&[], "C"),
            vec![
                query("http", Reply::Error("xdg-mime missing")),
                query("https", output("librewolf.desktop\n")),
            ],
        );
        let error = backend.current().unwrap_err().to_string();
        assert!(error.contains("HTTP=unavailable"));
        assert!(error.contains("xdg-mime missing"));
        assert!(error.contains("HTTPS=\"librewolf.desktop\""));
        assert!(script.borrow().is_empty());
    }

    #[test]
    fn malformed_query_output_is_an_error_not_a_false_default() {
        let fixture = Fixture::new();
        let (backend, script) = backend(
            fixture.discovery(&[], "C"),
            vec![
                query("http", output("one.desktop\ntwo.desktop\n")),
                query("https", Reply::Output(2, "", "query failed")),
            ],
        );
        let error = backend.current().unwrap_err().to_string();
        assert!(error.contains("invalid http desktop-file ID"));
        assert!(error.contains("query failed"));
        assert!(script.borrow().is_empty());
    }

    fn set_commands(
        id: &str,
        set: Reply,
        check: Reply,
        http: Reply,
        https: Reply,
    ) -> Vec<Expected> {
        vec![
            expected("xdg-settings", &["set", "default-web-browser", id], set),
            expected("xdg-settings", &["check", "default-web-browser", id], check),
            query("http", http),
            query("https", https),
        ]
    }

    #[test]
    fn setter_uses_exact_argv_and_requires_check_and_both_readbacks() {
        let fixture = Fixture::new();
        let id = "odd;$(never-execute).desktop";
        fixture.desktop("data", id, "Unfamiliar handler", "");
        let (backend, script) = backend(
            fixture.discovery(&["data"], "C"),
            set_commands(
                id,
                output(""),
                output("yes\n"),
                output("odd;$(never-execute).desktop\n"),
                output("odd;$(never-execute).desktop\n"),
            ),
        );
        backend
            .set_default(&backend.browsers().unwrap()[0], &|| false)
            .unwrap();
        assert!(script.borrow().is_empty());
    }

    #[test]
    fn setter_revalidates_discovery_before_running_any_command() {
        let fixture = Fixture::new();
        let path = fixture.desktop("data", "removed.desktop", "Removed", "");
        let (backend, script) = backend(fixture.discovery(&["data"], "C"), vec![]);
        let browser = backend.browsers().unwrap().remove(0);
        fs::remove_file(path).unwrap();
        assert!(backend
            .set_default(&browser, &|| false)
            .unwrap_err()
            .to_string()
            .contains("no longer an available"));
        assert!(script.borrow().is_empty());
    }

    #[test]
    fn check_no_is_failure_even_when_both_scheme_readbacks_match() {
        let fixture = Fixture::new();
        fixture.desktop("data", "brave.desktop", "Brave", "");
        let (backend, script) = backend(
            fixture.discovery(&["data"], "C"),
            set_commands(
                "brave.desktop",
                output(""),
                output("no\n"),
                output("brave.desktop"),
                output("brave.desktop"),
            ),
        );
        let error = backend
            .set_default(&backend.browsers().unwrap()[0], &|| false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("check failed"));
        assert!(error.contains("HTTP=\"brave.desktop\", HTTPS=\"brave.desktop\""));
        assert!(script.borrow().is_empty());
    }

    #[test]
    fn failed_set_is_an_error_even_when_check_and_readbacks_match() {
        let fixture = Fixture::new();
        fixture.desktop("data", "brave.desktop", "Brave", "");
        let (backend, script) = backend(
            fixture.discovery(&["data"], "C"),
            set_commands(
                "brave.desktop",
                Reply::Output(4, "", "set rejected"),
                output("yes"),
                output("brave.desktop"),
                output("brave.desktop"),
            ),
        );
        let error = backend
            .set_default(&backend.browsers().unwrap()[0], &|| false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("set failed"));
        assert!(error.contains("set rejected"));
        assert!(error.contains("HTTP=\"brave.desktop\", HTTPS=\"brave.desktop\""));
        assert!(script.borrow().is_empty());
    }

    #[test]
    fn failed_verification_query_prevents_success_after_set_and_check() {
        let fixture = Fixture::new();
        fixture.desktop("data", "brave.desktop", "Brave", "");
        let (backend, script) = backend(
            fixture.discovery(&["data"], "C"),
            set_commands(
                "brave.desktop",
                output(""),
                output("yes"),
                Reply::Output(4, "", "readback failed"),
                output("brave.desktop"),
            ),
        );
        let error = backend
            .set_default(&backend.browsers().unwrap()[0], &|| false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("HTTP=unavailable"));
        assert!(error.contains("readback failed"));
        assert!(error.contains("HTTPS=\"brave.desktop\""));
        assert!(script.borrow().is_empty());
    }

    #[test]
    fn setter_partial_failure_reports_truthful_mixed_state() {
        let fixture = Fixture::new();
        fixture.desktop("data", "chromium.desktop", "Chromium", "");
        let (backend, script) = backend(
            fixture.discovery(&["data"], "C"),
            set_commands(
                "chromium.desktop",
                Reply::Output(4, "", "session refused"),
                output("no"),
                output("chromium.desktop"),
                output("firefox.desktop"),
            ),
        );
        let error = backend
            .set_default(&backend.browsers().unwrap()[0], &|| false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("session refused"));
        assert!(error.contains("HTTP=\"chromium.desktop\", HTTPS=\"firefox.desktop\""));
        assert!(error.contains("not atomic"));
        assert!(script.borrow().is_empty());
    }

    #[test]
    fn successful_set_and_check_with_readback_mismatch_is_failure() {
        let fixture = Fixture::new();
        fixture.desktop("data", "librewolf.desktop", "LibreWolf", "");
        let (backend, script) = backend(
            fixture.discovery(&["data"], "C"),
            set_commands(
                "librewolf.desktop",
                output(""),
                output("yes"),
                output("librewolf.desktop"),
                output("firefox.desktop"),
            ),
        );
        let error = backend
            .set_default(&backend.browsers().unwrap()[0], &|| false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("did not both match"));
        assert!(error.contains("HTTPS=\"firefox.desktop\""));
        assert!(script.borrow().is_empty());
    }

    #[test]
    fn missing_tools_and_failed_readback_remain_errors() {
        let fixture = Fixture::new();
        fixture.desktop("data", "firefox.desktop", "Firefox", "");
        let (backend, script) = backend(
            fixture.discovery(&["data"], "C"),
            set_commands(
                "firefox.desktop",
                Reply::Error("xdg-settings missing: install xdg-utils"),
                Reply::Output(3, "", "no desktop session"),
                Reply::Error("xdg-mime missing"),
                output("firefox.desktop"),
            ),
        );
        let error = backend
            .set_default(&backend.browsers().unwrap()[0], &|| false)
            .unwrap_err()
            .to_string();
        assert!(error.contains("install xdg-utils"));
        assert!(error.contains("no desktop session"));
        assert!(error.contains("HTTP=unavailable"));
        assert!(error.contains("HTTPS=\"firefox.desktop\""));
        assert!(script.borrow().is_empty());
    }
}
