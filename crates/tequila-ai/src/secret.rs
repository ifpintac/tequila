//! Keeping API keys out of files anyone could read.
//!
//! On Windows a key is encrypted with the Data Protection API, which ties it
//! to this Windows account on this computer: the settings file can be copied,
//! backed up or synced, and the key in it is useless anywhere else. No
//! password to invent, nothing to type at start-up.
//!
//! On a Mac the key is kept in the login Keychain, where the Mac keeps every
//! password, and the settings file holds only which Keychain item it is.
//!
//! Elsewhere there is no equivalent without a keyring dependency, so the key
//! is stored merely encoded and marked as such — and the settings screen says
//! so, and offers an environment variable instead.

/// Protect a key for storage. The result is safe to write to a file.
pub fn protect(key: &str) -> Result<String, String> {
    match platform::protect(key.as_bytes()) {
        Ok(bytes) => Ok(format!("{}{}", platform::PREFIX, crate::base64::encode(&bytes))),
        // A Keychain can be locked or out of reach - a session with no one
        // logged in, a build machine - and a key it would not take is kept
        // encoded, as on a system with no keyring, rather than lost.
        #[cfg(target_os = "macos")]
        Err(why) => {
            eprintln!("Tequila: {why}; the key is stored encoded instead");
            Ok(format!("plain:{}", crate::base64::encode(key.as_bytes())))
        }
        #[cfg(not(target_os = "macos"))]
        Err(why) => Err(why),
    }
}

/// Recover a key protected by [`protect`].
pub fn unprotect(stored: &str) -> Result<String, String> {
    let (prefix, body) = stored
        .split_once(':')
        .ok_or("the stored key is not in a known form")?;
    let bytes = crate::base64::decode(body).ok_or("the stored key is damaged")?;
    let kind = format!("{prefix}:");
    let plain = if kind == platform::PREFIX {
        platform::unprotect(&bytes)?
    } else if kind == "plain:" {
        bytes
    } else {
        return Err(format!("the stored key was protected by another system ({prefix})"));
    };
    String::from_utf8(plain).map_err(|_| "the stored key is damaged".to_string())
}

/// Whether keys are encrypted on this platform, for the settings screen.
pub fn encrypted() -> bool {
    platform::PREFIX != "plain:"
}

/// Where this platform keeps a key, for the settings screen.
pub fn kept() -> &'static str {
    if cfg!(windows) {
        "API keys are encrypted for your Windows account, and are sent only to the service they belong to."
    } else if cfg!(target_os = "macos") {
        "API keys are kept in your Mac's Keychain, and are sent only to the service they belong to."
    } else {
        "API keys are stored encoded but not encrypted on this system; an environment variable is safer."
    }
}

#[cfg(windows)]
mod platform {
    use std::ffi::c_void;

    pub const PREFIX: &str = "dpapi:";

    #[repr(C)]
    struct Blob {
        size: u32,
        data: *mut u8,
    }

    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            input: *const Blob,
            description: *const u16,
            entropy: *const Blob,
            reserved: *mut c_void,
            prompt: *mut c_void,
            flags: u32,
            output: *mut Blob,
        ) -> i32;
        fn CryptUnprotectData(
            input: *const Blob,
            description: *mut *mut u16,
            entropy: *const Blob,
            reserved: *mut c_void,
            prompt: *mut c_void,
            flags: u32,
            output: *mut Blob,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(memory: *mut c_void) -> *mut c_void;
    }

    /// Mixed in so a blob protected by Tequila is not accepted by — or
    /// readable as — another program's data under the same account.
    const ENTROPY: &[u8] = b"Tequila assistant provider key";

    fn run(
        input: &[u8],
        call: impl FnOnce(*const Blob, *const Blob, *mut Blob) -> i32,
    ) -> Result<Vec<u8>, String> {
        let input = Blob { size: input.len() as u32, data: input.as_ptr() as *mut u8 };
        let entropy = Blob { size: ENTROPY.len() as u32, data: ENTROPY.as_ptr() as *mut u8 };
        let mut output = Blob { size: 0, data: std::ptr::null_mut() };
        if call(&input, &entropy, &mut output) == 0 {
            return Err("Windows could not protect or read the key for this account".into());
        }
        // SAFETY: on success the call has filled `output` with a buffer of
        // `size` bytes that it allocated with LocalAlloc; it is copied out
        // and then freed exactly once, as the documentation requires.
        let bytes = unsafe { std::slice::from_raw_parts(output.data, output.size as usize).to_vec() };
        unsafe { LocalFree(output.data as *mut c_void) };
        Ok(bytes)
    }

    pub fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
        // SAFETY: every pointer is to a live Blob whose buffer outlives the
        // call; the optional arguments are null, which the API allows.
        run(plain, |input, entropy, output| unsafe {
            CryptProtectData(input, std::ptr::null(), entropy, std::ptr::null_mut(), std::ptr::null_mut(), CRYPTPROTECT_UI_FORBIDDEN, output)
        })
    }

    pub fn unprotect(sealed: &[u8]) -> Result<Vec<u8>, String> {
        // SAFETY: as for `protect`; the description out-pointer is null, so
        // there is nothing else to free.
        run(sealed, |input, entropy, output| unsafe {
            CryptUnprotectData(input, std::ptr::null_mut(), entropy, std::ptr::null_mut(), std::ptr::null_mut(), CRYPTPROTECT_UI_FORBIDDEN, output)
        })
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use security_framework::passwords::{get_generic_password, set_generic_password};

    pub const PREFIX: &str = "keychain:";
    /// The Keychain items' service, as Keychain Access lists them.
    const SERVICE: &str = "Tequila";

    pub fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
        // An item a key, named for its digest: the settings are written
        // every time they change, and the same key saved again is the same
        // item, not one more.
        let account = format!("assistant-key-{:016x}", digest(plain));
        set_generic_password(SERVICE, &account, plain).map_err(|e| format!("the Keychain would not keep the key ({e})"))?;
        Ok(account.into_bytes())
    }

    pub fn unprotect(sealed: &[u8]) -> Result<Vec<u8>, String> {
        let account = std::str::from_utf8(sealed).map_err(|_| "the stored key is damaged".to_string())?;
        get_generic_password(SERVICE, account).map_err(|e| format!("the Keychain has no such key for Tequila ({e})"))
    }

    /// FNV-1a: a name for the item, not a protection of anything. An API
    /// key has far more than 64 bits in it, so the name gives nothing away.
    fn digest(bytes: &[u8]) -> u64 {
        bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| (hash ^ *byte as u64).wrapping_mul(0x0100_0000_01b3))
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    pub const PREFIX: &str = "plain:";

    pub fn protect(plain: &[u8]) -> Result<Vec<u8>, String> {
        Ok(plain.to_vec())
    }

    pub fn unprotect(sealed: &[u8]) -> Result<Vec<u8>, String> {
        Ok(sealed.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_round_trips() {
        let stored = protect("sk-test-1234").unwrap();
        assert_eq!(unprotect(&stored).unwrap(), "sk-test-1234");
    }

    /// What is written to the settings file is not the key.
    #[cfg(windows)]
    #[test]
    fn a_stored_key_is_not_readable_as_itself() {
        let stored = protect("sk-test-1234").unwrap();
        assert!(stored.starts_with("dpapi:"));
        assert!(!stored.contains("sk-test"));
        let decoded = crate::base64::decode(stored.trim_start_matches("dpapi:")).unwrap();
        assert!(!decoded.windows(7).any(|w| w == b"sk-test"), "the key is in the blob in the clear");
    }

    /// A key the Keychain kept is not in the file; one it would not take
    /// (a locked Keychain on a build machine) comes back all the same.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_mac_keeps_the_key_in_its_keychain_or_says_so() {
        let stored = protect("sk-test-mac-5678").unwrap();
        if stored.starts_with("keychain:") {
            assert!(!stored.contains("sk-test"));
        } else {
            assert!(stored.starts_with("plain:"), "{stored}");
        }
        assert_eq!(unprotect(&stored).unwrap(), "sk-test-mac-5678");
        assert_eq!(protect("sk-test-mac-5678").unwrap(), stored, "the same key is the same item");
    }

    #[test]
    fn damaged_keys_are_refused_not_guessed() {
        assert!(unprotect("nonsense").is_err());
        assert!(unprotect("other:AAAA").is_err());
    }
}
