//! The Gemini API key lives in the OS keychain, not in settings.json.
//!
//! `app_settings` keeps `google_transcribe.api_key` as the in-memory carrier
//! only: [`prime`] reads the store once at the first settings load (the load
//! puts the stored key back into the settings), [`sync`] stores or clears it
//! at every save, and [`holds`] tells the write path whether the file may
//! leave it out. While the keychain is unavailable nothing is lost — the key
//! stays in the file, saves keep working, and a later sync moves it in.
//!
//! macOS holds it in the login keychain, Windows in Credential Manager, and
//! Linux in the Secret Service of the session bus (same user), so no crypto
//! features are asked of the keyring crate.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

/// What each settings path last saw the store hold, so [`sync`] and
/// [`holds`] compare against what is actually stored. Keyed by the settings
/// path — one process serves one, but tests use several and they must not
/// interfere, as `UNPARSEABLE` does not.
static BASELINES: LazyLock<Mutex<HashMap<PathBuf, Option<String>>>> =
    LazyLock::new(Default::default);

/// Reads the store for `scope` and records what it holds: the baseline a
/// later [`sync`] and [`holds`] compare against, kept so a save changes
/// nothing when the key has not changed. Returns the stored key; an error
/// leaves the baseline unset, so the file keeps carrying whatever it has.
pub(crate) fn prime(scope: &Path) -> Result<Option<String>, String> {
    if let Some(stored) = baselines().get(scope) {
        return Ok(stored.clone());
    }
    let stored = imp::read(scope)?;
    baselines().insert(scope.to_path_buf(), stored.clone());
    Ok(stored)
}

/// Makes the store hold `key` for `scope`, storing or clearing only when it
/// differs from what [`prime`] last saw. An unavailable store answers an
/// error, and the caller keeps the key in the file instead.
pub(crate) fn sync(scope: &Path, key: Option<&str>) -> Result<(), String> {
    let key = real_key(key);
    if let Some(stored) = baselines().get(scope)
        && stored.as_deref() == key
    {
        return Ok(());
    }
    let stored = imp::read(scope)?;
    baselines().insert(scope.to_path_buf(), stored.clone());
    if stored.as_deref() != key {
        imp::write(scope, key)?;
        baselines().insert(scope.to_path_buf(), key.map(str::to_owned));
    }
    Ok(())
}

/// Whether the store already holds `key` for `scope`, so the settings file
/// can leave it out. A scope never primed is known to hold no key.
pub(crate) fn holds(scope: &Path, key: Option<&str>) -> bool {
    let key = real_key(key);
    match baselines().get(scope) {
        Some(stored) => stored.as_deref() == key,
        None => key.is_none(),
    }
}

/// An empty key is no key, as `gemini::set_api_key` also treats it.
fn real_key(key: Option<&str>) -> Option<&str> {
    key.filter(|key| !key.trim().is_empty())
}

fn baselines() -> std::sync::MutexGuard<'static, HashMap<PathBuf, Option<String>>> {
    BASELINES.lock().unwrap_or_else(|error| error.into_inner())
}

#[cfg(not(test))]
mod imp {
    use std::path::Path;

    /// The keyring service Voinc stores the key under, and the account,
    /// shared by every OS backend.
    const SERVICE: &str = "io.voinc.voinc";
    const ACCOUNT: &str = "gemini-api-key";

    fn entry() -> Result<keyring::Entry, String> {
        keyring::Entry::new(SERVICE, ACCOUNT).map_err(|error| error.to_string())
    }

    pub(super) fn read(_scope: &Path) -> Result<Option<String>, String> {
        match entry()?.get_password() {
            Ok(key) => Ok(Some(key)),
            // The store simply has no key yet.
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }

    pub(super) fn write(_scope: &Path, key: Option<&str>) -> Result<(), String> {
        let entry = entry()?;
        match key {
            Some(key) => entry.set_password(key).map_err(|error| error.to_string()),
            None => match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(error) => Err(error.to_string()),
            },
        }
    }
}

#[cfg(test)]
mod imp {
    use std::collections::{HashMap, HashSet};
    use std::path::{Path, PathBuf};
    use std::sync::{LazyLock, Mutex};

    /// What the store holds, keyed by scope: an in-memory stand-in, so no
    /// test ever touches the real keychain.
    static STORES: LazyLock<Mutex<HashMap<PathBuf, Option<String>>>> =
        LazyLock::new(Default::default);
    /// Scopes whose store operations fail, as a locked or missing keychain
    /// would.
    static FAILING: LazyLock<Mutex<HashSet<PathBuf>>> = LazyLock::new(Default::default);

    fn failing(scope: &Path) -> bool {
        FAILING
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .contains(scope)
    }

    pub(super) fn read(scope: &Path) -> Result<Option<String>, String> {
        if failing(scope) {
            return Err("the keychain is unavailable".into());
        }
        Ok(STORES
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(scope)
            .cloned()
            .unwrap_or_default())
    }

    pub(super) fn write(scope: &Path, key: Option<&str>) -> Result<(), String> {
        if failing(scope) {
            return Err("the keychain is unavailable".into());
        }
        STORES
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .insert(scope.to_path_buf(), key.map(str::to_owned));
        Ok(())
    }

    /// Makes every store operation for `scope` fail until asked not to.
    pub(super) fn fail(scope: &Path, on: bool) {
        let mut failing = FAILING.lock().unwrap_or_else(|error| error.into_inner());
        if on {
            failing.insert(scope.to_path_buf());
        } else {
            failing.remove(scope);
        }
    }
}

/// Test-only: makes the keychain unavailable for `scope`.
#[cfg(test)]
pub(crate) fn test_fail(scope: &Path, on: bool) {
    imp::fail(scope, on);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scope no other test uses; the recorder never touches the disk, so
    /// a made-up path is enough.
    fn scope(name: &str) -> PathBuf {
        PathBuf::from(format!("key-store-test-{name}"))
    }

    #[test]
    fn a_prime_records_what_the_store_holds() {
        let scope = scope("prime");
        assert_eq!(prime(&scope).unwrap(), None, "an untouched store is empty");
        sync(&scope, Some("k")).unwrap();
        assert_eq!(prime(&scope).unwrap().as_deref(), Some("k"));
    }

    #[test]
    fn a_sync_stores_a_new_key_and_changes_nothing_after() {
        let scope = scope("sync");
        sync(&scope, Some("k")).unwrap();
        sync(&scope, Some("k")).unwrap();
        assert_eq!(imp::read(&scope).unwrap().as_deref(), Some("k"));
        sync(&scope, Some("k2")).unwrap();
        assert_eq!(imp::read(&scope).unwrap().as_deref(), Some("k2"));
    }

    #[test]
    fn clearing_a_key_clears_the_store() {
        let scope = scope("clear");
        sync(&scope, Some("k")).unwrap();
        sync(&scope, None).unwrap();
        assert_eq!(imp::read(&scope).unwrap(), None);
        // Clearing an already empty store stays a success.
        sync(&scope, None).unwrap();
    }

    #[test]
    fn an_unavailable_store_keeps_the_key_in_the_file() {
        let scope = scope("failing");
        test_fail(&scope, true);
        assert!(prime(&scope).is_err());
        assert!(sync(&scope, Some("k")).is_err());
        // The write path then leaves the key in the file: `holds` is false
        // for it, and true for no key.
        assert!(!holds(&scope, Some("k")));
        assert!(holds(&scope, None));
        test_fail(&scope, false);
        // It heals: the next sync stores the key.
        sync(&scope, Some("k")).unwrap();
        assert!(holds(&scope, Some("k")));
    }

    #[test]
    fn an_empty_key_is_no_key() {
        let scope = scope("empty");
        sync(&scope, Some("   ")).unwrap();
        assert_eq!(imp::read(&scope).unwrap(), None, "nothing was stored");
        assert!(holds(&scope, Some("  ")), "the file leaves it out too");
    }
}
