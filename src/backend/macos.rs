//! Registered HTTP/HTTPS handlers through NSWorkspace (macOS 12 and later).

use super::{Backend, Browser, CurrentDefaults};
use anyhow::{anyhow, bail, Context, Result};
use block2::RcBlock;
use objc2::{available, rc::autoreleasepool, MainThreadMarker, Message};
use objc2_app_kit::NSWorkspace;
use objc2_foundation::{
    NSBundle, NSDate, NSDefaultRunLoopMode, NSError, NSRunLoop, NSString, NSURL,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const CONFIRMATION_TIMEOUT: Duration = Duration::from_secs(120);

pub struct SystemBackend {
    workspace: objc2::rc::Retained<NSWorkspace>,
    // The synchronous Backend contract must service Cocoa's main run loop.
    _main_thread: MainThreadMarker,
}

impl SystemBackend {
    pub fn new() -> Result<Self> {
        if !available!(macos = 12.0) {
            bail!("The macOS browser backend requires macOS 12 or later");
        }
        let main_thread = MainThreadMarker::new()
            .context("The macOS browser backend must run on the Cocoa main thread")?;
        Ok(autoreleasepool(|_| Self {
            workspace: NSWorkspace::sharedWorkspace(),
            _main_thread: main_thread,
        }))
    }

    fn scheme_browsers(&self, scheme: &str) -> Result<Vec<Browser>> {
        let url = sample_url(scheme)?;
        let urls = self.workspace.URLsForApplicationsToOpenURL(&url);
        // Launch Services can contain stale registrations; only installed bundles qualify.
        Ok((0..urls.count())
            .filter_map(|index| installed_browser(&urls.objectAtIndex(index)))
            .collect())
    }

    fn current_scheme(&self, scheme: &str) -> Result<Option<String>> {
        let url = sample_url(scheme)?;
        self.workspace
            .URLForApplicationToOpenURL(&url)
            .map(|url| app_identity(&url))
            .transpose()
            .with_context(|| format!("Could not resolve the current {scheme} application"))
    }

    fn request_scheme(&self, application: &NSURL, scheme: &str) -> Result<()> {
        let scheme_string = NSString::from_str(scheme);
        let completion = Completion::default();
        let callback = completion_block(&completion, application, &scheme_string);
        self.workspace
            .setDefaultApplicationAtURL_toOpenURLsWithScheme_completionHandler(
                application,
                &scheme_string,
                Some(&callback),
            );
        let run_loop = NSRunLoop::mainRunLoop();
        wait_for_completion(&completion, CONFIRMATION_TIMEOUT, |slice| {
            autoreleasepool(|_| {
                let until = NSDate::dateWithTimeIntervalSinceNow(slice.as_secs_f64());
                // SAFETY: Foundation's exported immutable run-loop mode is valid on macOS.
                let serviced = unsafe { run_loop.runMode_beforeDate(NSDefaultRunLoopMode, &until) };
                if !serviced {
                    // An empty run loop returns immediately; avoid spinning while a callback
                    // on a different queue is pending. Never sleep longer than this slice.
                    std::thread::sleep(slice.min(Duration::from_millis(10)));
                }
            });
        })
        .with_context(|| format!("macOS could not finish setting {scheme}"))
    }

    fn change_error(&self, error: anyhow::Error) -> anyhow::Error {
        match self.current() {
            Ok(current) => anyhow!(
                "{error:#}. Defaults are now HTTP={}, HTTPS={}. Changes are not atomic; a partial change may have occurred.",
                current.http.as_deref().unwrap_or("(none)"),
                current.https.as_deref().unwrap_or("(none)")
            ),
            Err(read_error) => anyhow!(
                "{error:#}. Reading the resulting HTTP/HTTPS defaults also failed: {read_error:#}. The actual state is unknown; a partial change may have occurred."
            ),
        }
    }
}

impl Backend for SystemBackend {
    fn browsers(&self) -> Result<Vec<Browser>> {
        autoreleasepool(|_| {
            Ok(common_browsers(
                self.scheme_browsers("http")?,
                self.scheme_browsers("https")?,
            ))
        })
    }

    fn current(&self) -> Result<CurrentDefaults> {
        autoreleasepool(|_| {
            Ok(CurrentDefaults {
                http: self.current_scheme("http")?,
                https: self.current_scheme("https")?,
            })
        })
    }

    fn set_default(&self, browser: &Browser) -> Result<()> {
        // Re-discover instead of trusting a stale picker entry or caller-supplied path.
        let installed = self.browsers()?;
        let chosen = installed
            .iter()
            .find(|candidate| candidate.id == browser.id)
            .context("The selected application is no longer an installed HTTP and HTTPS handler; run defbrow list again")?;
        autoreleasepool(|_| {
            let application = NSURL::fileURLWithPath(&NSString::from_str(&chosen.id));
            for scheme in ["http", "https"] {
                if let Err(error) = self.request_scheme(&application, scheme) {
                    return Err(self.change_error(error));
                }
            }
            let current = self.current().map_err(|error| self.change_error(error))?;
            verify_defaults(&chosen.id, &current)
        })
    }
}

fn sample_url(scheme: &str) -> Result<objc2::rc::Retained<NSURL>> {
    // Query only; this URL is never opened or fetched.
    NSURL::URLWithString(&NSString::from_str(&format!("{scheme}://example.invalid/")))
        .context("Could not construct the handler query URL")
}

fn app_identity(url: &NSURL) -> Result<String> {
    let path = url
        .path()
        .context("The application URL has no filesystem path")?;
    canonical_app_path(Path::new(&path.to_string()))
}

fn canonical_app_path(path: &Path) -> Result<String> {
    let canonical = path.canonicalize().with_context(|| {
        format!(
            "The application is missing or inaccessible: {}",
            path.display()
        )
    })?;
    if !canonical.is_dir() {
        bail!(
            "The application is not a bundle directory: {}",
            canonical.display()
        );
    }
    canonical
        .to_str()
        .map(str::to_owned)
        .context("The application path cannot be represented as a UTF-8 handler identity")
}

fn installed_browser(url: &NSURL) -> Option<Browser> {
    // Paths, rather than bundle IDs, preserve distinct installed copies. The same
    // canonicalization is used for current(), so symlink aliases share an identity.
    let id = app_identity(url).ok()?;
    let canonical_url = NSURL::fileURLWithPath(&NSString::from_str(&id));
    let bundle = NSBundle::bundleWithURL(&canonical_url)?;
    let executable = bundle.executableURL()?.path()?.to_string();
    if !Path::new(&executable).is_file() {
        return None;
    }
    let metadata_name = |key| {
        let value = bundle.objectForInfoDictionaryKey(&NSString::from_str(key))?;
        let name = value.downcast_ref::<NSString>()?.to_string();
        (!name.trim().is_empty()).then_some(name)
    };
    let display_name = metadata_name("CFBundleDisplayName");
    let bundle_name = metadata_name("CFBundleName");
    let bundle_id = bundle.bundleIdentifier().map(|value| value.to_string());
    if !eligible_browser_install(
        Path::new(&id),
        bundle_id.as_deref(),
        bundle_name.as_deref(),
        display_name.as_deref(),
    ) {
        return None;
    }
    let name = display_name.or(bundle_name).unwrap_or_else(|| {
        Path::new(&id)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    });
    let detail = match bundle_id {
        Some(bundle_id) => format!("{bundle_id} — {id}"),
        None => id.clone(),
    };
    Some(Browser { id, name, detail })
}

// Eligibility applies only to candidates, never current(). Canonical paths prevent
// aliases from admitting cached copies; set_default() reuses this discovery gate.
// Reject dedicated cache/staging locations, not browser families or release channels.
fn eligible_browser_install(
    path: &Path,
    bundle_id: Option<&str>,
    bundle_name: Option<&str>,
    display_name: Option<&str>,
) -> bool {
    if bundle_id == Some("com.google.chrome.for.testing")
        || [
            bundle_name,
            display_name,
            path.file_stem().and_then(|s| s.to_str()),
        ]
        .into_iter()
        .flatten()
        .any(|name| name == "Google Chrome for Testing")
    {
        return false;
    }

    for ancestor in path.parent().into_iter().flat_map(Path::ancestors) {
        if ancestor.file_name().is_some_and(|name| {
            [
                ".cloakbrowser",
                ".fragrantica-archiver",
                ".cache",
                "ms-playwright",
                ".puppeteer",
            ]
            .iter()
            .any(|excluded| name == *excluded)
        }) || [
            "Library/Caches",
            ".agent-browser/browsers",
            ".omp/puppeteer",
            "puppeteer/.local-chromium",
            "puppeteer-core/.local-chromium",
            "puppeteer/.local-browsers",
            "puppeteer-core/.local-browsers",
        ]
        .iter()
        .any(|suffix| ancestor.ends_with(suffix))
        {
            return false;
        }
    }

    if ["/tmp", "/private/tmp", "/var/tmp", "/private/var/tmp"]
        .iter()
        .any(|root| path.starts_with(root))
    {
        return false;
    }
    // Darwin's per-user temporary/cache roots: /[private/]var/folders/<shard>/<user>/T|C.
    for root in ["/var/folders", "/private/var/folders"] {
        if let Ok(relative) = path.strip_prefix(root) {
            if relative
                .iter()
                .nth(2)
                .is_some_and(|part| part == "T" || part == "C")
            {
                return false;
            }
        }
    }

    // Mozilla updater's downloaded staging bundle is not a user-installed copy.
    let parts: Vec<_> = path.iter().collect();
    if let [.., update_info, applications, _, updates, stage, updated] = parts.as_slice() {
        if *update_info == "UpdateInfo"
            && *applications == "Applications"
            && *updates == "updates"
            && *updated == "Updated.app"
            && stage.to_str().is_some_and(|stage| {
                !stage.is_empty() && stage.bytes().all(|byte| byte.is_ascii_digit())
            })
        {
            return false;
        }
    }
    true
}

fn common_browsers(http: Vec<Browser>, https: Vec<Browser>) -> Vec<Browser> {
    let https_ids: BTreeSet<_> = https.into_iter().map(|browser| browser.id).collect();
    let mut common = BTreeMap::new();
    for browser in http {
        if https_ids.contains(&browser.id) {
            common.entry(browser.id.clone()).or_insert(browser);
        }
    }
    let mut browsers: Vec<_> = common.into_values().collect();
    browsers.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    browsers
}

fn verify_defaults(id: &str, current: &CurrentDefaults) -> Result<()> {
    if current.http.as_deref() != Some(id) || current.https.as_deref() != Some(id) {
        bail!(
            "macOS did not confirm both defaults for {id}: HTTP={}, HTTPS={}. A partial change may have occurred; check System Settings and run defbrow current.",
            current.http.as_deref().unwrap_or("(none)"),
            current.https.as_deref().unwrap_or("(none)")
        );
    }
    Ok(())
}

#[derive(Clone, Default)]
struct Completion(Arc<Mutex<Option<std::result::Result<(), String>>>>);

impl Completion {
    fn record(&self, result: std::result::Result<(), String>) {
        let mut state = self.0.lock().unwrap_or_else(|poison| poison.into_inner());
        if state.is_none() {
            *state = Some(result);
        }
    }

    fn result(&self) -> Option<std::result::Result<(), String>> {
        self.0
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }
}

fn completion_block(
    completion: &Completion,
    application: &NSURL,
    scheme: &NSString,
) -> RcBlock<dyn Fn(*mut NSError)> {
    let completion = completion.clone();
    let application = application.retain();
    let scheme = scheme.retain();
    RcBlock::new(move |error: *mut NSError| {
        // NSWorkspace copies the asynchronous block. It owns all captures, including
        // the request's URLs/strings and shared state, even after our wait times out.
        let _keep_alive = (&application, &scheme);
        let result = autoreleasepool(|_| {
            // SAFETY: NSWorkspace passes either null or an NSError valid throughout
            // this callback. Copy its strings here; never store the borrowed pointer.
            match unsafe { error.as_ref() } {
                None => Ok(()),
                Some(error) => Err(format!(
                    "{} ({} code {})",
                    error.localizedDescription(),
                    error.domain(),
                    error.code()
                )),
            }
        });
        completion.record(result);
    })
}

fn wait_for_completion(
    completion: &Completion,
    timeout: Duration,
    mut service_run_loop: impl FnMut(Duration),
) -> Result<()> {
    let started = Instant::now();
    loop {
        if let Some(result) = completion.result() {
            return result.map_err(anyhow::Error::msg);
        }
        let remaining = timeout.saturating_sub(started.elapsed());
        if remaining.is_zero() {
            bail!("Timed out awaiting macOS confirmation. Respond to any OS prompt, check System Settings, then run defbrow current before retrying. The pending request cannot be cancelled and may still change the default later");
        }
        service_run_loop(remaining.min(Duration::from_millis(50)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn browser(id: &str, name: &str) -> Browser {
        Browser {
            id: id.into(),
            name: name.into(),
            detail: id.into(),
        }
    }

    #[test]
    fn eligibility_excludes_dedicated_automation_caches_and_temporary_roots() {
        for path in [
            "/Users/milo/.cloakbrowser/chromium-145.0.7632.109.2/Chromium.app",
            "/Users/milo/Projects/fragtrack/.fragrantica-archiver/chromium-145.0.7632.109.2/Chromium.app",
            "/Users/milo/.cache/rod/browser/chromium-1321438/Chromium.app",
            "/Users/milo/Library/Caches/ms-playwright/firefox-1538/firefox/Nightly.app",
            "/Users/milo/Library/Caches/ms-playwright/chromium-1234/chrome-mac-arm64/Google Chrome for Testing.app",
            "/opt/downloads/ms-playwright/chromium/Chromium.app",
            "/Users/milo/.agent-browser/browsers/chrome-151.0.7922.47/Google Chrome for Testing.app",
            "/Users/milo/.omp/puppeteer/chrome/mac_arm-150.0.7871.24/chrome-mac-arm64/Google Chrome for Testing.app",
            "/Users/milo/.puppeteer/chrome/Chromium.app",
            "/Users/milo/.vscode/extensions/yzane.markdown-pdf-1.5.0/node_modules/puppeteer-core/.local-chromium/mac-722234/chrome-mac/Chromium.app",
            "/opt/node_modules/puppeteer/.local-chromium/chrome/Chromium.app",
            "/opt/node_modules/puppeteer-core/.local-browsers/chrome/Chromium.app",
            "/opt/node_modules/puppeteer/.local-browsers/chrome/Chromium.app",
            "/tmp/browser/Chromium.app",
            "/private/tmp/browser/Chromium.app",
            "/var/tmp/browser/Chromium.app",
            "/private/var/tmp/browser/Chromium.app",
            "/var/folders/p0/user/T/browser/Chromium.app",
            "/private/var/folders/p0/user/C/browser/Chromium.app",
        ] {
            assert!(
                !eligible_browser_install(Path::new(path), None, None, None),
                "unexpected eligible cache: {path}"
            );
        }
    }

    #[test]
    fn eligibility_excludes_only_precise_chrome_for_testing_metadata_or_name() {
        let ordinary_path = Path::new("/opt/Browsers/Chromium.app");
        for (id, name, display) in [
            (Some("com.google.chrome.for.testing"), None, None),
            (None, Some("Google Chrome for Testing"), None),
            (None, None, Some("Google Chrome for Testing")),
        ] {
            assert!(!eligible_browser_install(ordinary_path, id, name, display));
        }
        assert!(!eligible_browser_install(
            Path::new("/Applications/Google Chrome for Testing.app"),
            None,
            None,
            None
        ));
        assert!(eligible_browser_install(
            ordinary_path,
            Some("com.google.chrome.for.testing.custom"),
            Some("Google Chrome for Testing Custom"),
            None
        ));
    }

    #[test]
    fn eligibility_preserves_normal_browsers_forks_channels_and_custom_locations() {
        for (path, id, name) in [
            ("/Applications/Safari.app", "com.apple.Safari", "Safari"),
            (
                "/System/Volumes/Preboot/Cryptexes/App/System/Applications/Safari.app",
                "com.apple.Safari",
                "Safari",
            ),
            (
                "/Applications/Brave Browser.app",
                "com.brave.Browser",
                "Brave Browser",
            ),
            (
                "/Users/milo/Applications/LibreWolf.app",
                "io.gitlab.librewolf-community",
                "LibreWolf",
            ),
            (
                "/Applications/Chromium.app",
                "org.chromium.Chromium",
                "Chromium",
            ),
            (
                "/Users/milo/Browsers/Firefox Nightly.app",
                "org.mozilla.nightlyunofficial",
                "Nightly",
            ),
            (
                "/Volumes/Browser Tools/My Fork.app",
                "dev.custom.browser",
                "My Fork",
            ),
            (
                "/opt/browsers/Google Chrome Beta.app",
                "com.google.Chrome.beta",
                "Google Chrome Beta",
            ),
            (
                "/Users/milo/.personal-browser/My Fork.app",
                "dev.custom.browser",
                "My Fork",
            ),
            (
                "/Applications/Mullvad Browser.app",
                "net.mullvad.mullvadbrowser",
                "Mullvad Browser",
            ),
        ] {
            assert!(
                eligible_browser_install(Path::new(path), Some(id), Some(name), None),
                "unexpected ineligible installation: {path}"
            );
        }
        for path in [
            "/Users/milo/.cloakbrowser-custom/Chromium.app",
            "/Users/milo/.fragrantica-archiver-custom/Chromium.app",
            "/Users/milo/.cache-custom/Chromium.app",
            "/Users/milo/Library/Caches-custom/Chromium.app",
            "/Users/milo/Library/My Caches/Chromium.app",
            "/Users/milo/ms-playwright-custom/Chromium.app",
            "/Users/milo/.puppeteer-custom/Chromium.app",
            "/Users/milo/.agent-browser/personal/Chromium.app",
            "/Users/milo/.omp/personal/Chromium.app",
            "/Users/milo/puppeteer/Browsers/Chromium.app",
            "/Users/milo/Browsers/.local-chromium/Chromium.app",
            "/tmp-browser/Chromium.app",
            "/private/var/tmp-browser/Chromium.app",
            "/private/var/folders/p0/user/Browsers/Chromium.app",
            "/Applications/Google Chrome for Testing Custom.app",
        ] {
            assert!(
                eligible_browser_install(Path::new(path), None, None, None),
                "{path}"
            );
        }
    }

    #[test]
    fn eligibility_excludes_precise_mozilla_update_staging_shape() {
        let staging = "/Users/milo/Library/Application Support/MullvadBrowser/UpdateInfo/Applications/Mullvad Browser/updates/0/Updated.app";
        assert!(!eligible_browser_install(
            Path::new(staging),
            None,
            None,
            None
        ));
        for path in [
            "/Users/milo/Browsers/Updated.app",
            "/Users/milo/UpdateInfo/Applications/My Fork/updates/custom/Updated.app",
            "/Users/milo/UpdateInfo/Applications/My Fork/updates/0/My Fork.app",
            "/Users/milo/UpdateInfo/Applications/My Fork/updates/0/Updated.app/My Fork.app",
            "/Users/milo/OtherInfo/Applications/My Fork/updates/0/Updated.app",
            "/Users/milo/UpdateInfo/Applications/My Fork/releases/0/Updated.app",
        ] {
            assert!(
                eligible_browser_install(Path::new(path), None, None, None),
                "{path}"
            );
        }
    }

    #[test]
    fn discovery_intersects_schemes_and_preserves_distinct_copies() {
        let copy1 = browser("/Applications/Unknown.app", "Unknown");
        let copy2 = browser("/Users/test/Unknown.app", "Unknown");
        let other = browser("/Applications/Other.app", "another fork");
        let http = vec![
            copy2.clone(),
            copy1.clone(),
            copy1.clone(),
            other.clone(),
            browser("http-only", "HTTP"),
        ];
        let https = vec![
            copy1.clone(),
            other.clone(),
            copy2.clone(),
            browser("https-only", "HTTPS"),
        ];
        assert_eq!(common_browsers(http, https), vec![other, copy1, copy2]);
    }

    #[test]
    fn missing_apps_and_files_are_not_bundle_identities() {
        let missing = std::env::temp_dir().join(format!("defbrow-missing-{}", std::process::id()));
        assert!(canonical_app_path(&missing).is_err());
        assert!(canonical_app_path(Path::new(file!())).is_err());
    }

    #[test]
    fn identity_canonicalizes_aliases() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir().join(format!(
            "defbrow-macos-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let app = root.join("Generic.app");
        std::fs::create_dir(&app).unwrap();
        let alias = root.join("Alias.app");
        symlink(&app, &alias).unwrap();
        let actual = canonical_app_path(&app).unwrap();
        let aliased = canonical_app_path(&alias).unwrap();
        std::fs::remove_dir_all(root).unwrap();
        assert_eq!(actual, aliased);
    }

    #[test]
    fn verification_rejects_partial_or_absent_defaults() {
        assert!(verify_defaults(
            "selected",
            &CurrentDefaults {
                http: Some("selected".into()),
                https: Some("selected".into())
            }
        )
        .is_ok());
        let partial = CurrentDefaults {
            http: Some("selected".into()),
            https: Some("previous".into()),
        };
        let error = verify_defaults("selected", &partial)
            .unwrap_err()
            .to_string();
        assert!(error.contains("HTTP=selected, HTTPS=previous"));
        assert!(verify_defaults("selected", &CurrentDefaults::default()).is_err());
    }

    #[test]
    fn completion_wait_services_callbacks_and_keeps_first_result() {
        let completion = Completion::default();
        wait_for_completion(&completion, Duration::from_secs(1), |_| {
            completion.record(Ok(()))
        })
        .unwrap();
        completion.record(Err("duplicate callback".into()));
        assert_eq!(completion.result(), Some(Ok(())));
    }

    #[test]
    fn callback_errors_are_preserved() {
        let completion = Completion::default();
        completion.record(Err("user rejected the request".into()));
        let error =
            wait_for_completion(&completion, Duration::ZERO, |_| panic!("already complete"))
                .unwrap_err();
        assert_eq!(error.to_string(), "user rejected the request");
    }

    #[test]
    fn timeout_does_not_invalidate_late_callback_state() {
        let completion = Completion::default();
        let late = completion.clone();
        let error =
            wait_for_completion(&completion, Duration::ZERO, |_| panic!("timeout")).unwrap_err();
        assert!(error
            .to_string()
            .contains("may still change the default later"));
        std::thread::spawn(move || late.record(Ok(())))
            .join()
            .unwrap();
        assert_eq!(completion.result(), Some(Ok(())));
    }

    #[test]
    fn native_block_copies_error_and_outlives_waiter_without_setting_defaults() {
        autoreleasepool(|_| {
            let completion = Completion::default();
            let application = NSURL::fileURLWithPath(&NSString::from_str("/fixture/Generic.app"));
            let scheme = NSString::from_str("http");
            let callback = completion_block(&completion, &application, &scheme);
            let copied = callback.clone();
            drop(callback);
            drop(application);
            drop(scheme);
            // SAFETY: No user-info dictionary is supplied, so no generic contents
            // can violate NSError's dictionary type requirements.
            let error = unsafe {
                NSError::errorWithDomain_code_userInfo(
                    &NSString::from_str("FixtureRejection"),
                    42,
                    None,
                )
            };
            copied.call((objc2::rc::Retained::as_ptr(&error) as *mut NSError,));
            drop(error);
            let result = completion.result().unwrap().unwrap_err();
            assert!(result.contains("FixtureRejection code 42"));
            copied.call((std::ptr::null_mut(),));
            assert_eq!(completion.result(), Some(Err(result)));
        });
    }
}
