//! Keeping API keys out of files anyone could read.
//!
//! On Windows a key is encrypted with the Data Protection API, which ties it
//! to this Windows account on this computer: the settings file can be copied,
//! backed up or synced, and the key in it is useless anywhere else. No
//! password to invent, nothing to type at start-up.
//!
//! Elsewhere there is no equivalent without a keyring dependency, so the key
//! is stored merely encoded and marked as such — and the settings screen says
//! so, and offers an environment variable instead.

/// Protect a key for storage. The result is safe to write to a file.
pub fn protect(key: &str) -> Result<String, String> {
    platform::protect(key.as_bytes()).map(|bytes| format!("{}{}", platform::PREFIX, crate::base64::encode(&bytes)))
}

/// Recover a key protected by [`protect`].
pub fn unprotect(stored: &str) -> Result<String, String> {
    let (prefix, body) = stored
        .split_once(':')
        .ok_or("the stored key is not in a known form")?;
    let bytes = crate::base64::decode(body).ok_or("the stored key is damaged")?;
    let plain = match format!("{prefix}:").as_str() {
        platform::PREFIX => platform::unprotect(&bytes)?,
        "plain:" => bytes,
        _ => return Err(format!("the stored key was protected by another system ({prefix})")),
    };
    String::from_utf8(plain).map_err(|_| "the stored key is damaged".to_string())
}

/// Whether keys are encrypted on this platform, for the settings screen.
pub fn encrypted() -> bool {
    platform::PREFIX != "plain:"
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

#[cfg(not(windows))]
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

    #[test]
    fn damaged_keys_are_refused_not_guessed() {
        assert!(unprotect("nonsense").is_err());
        assert!(unprotect("other:AAAA").is_err());
    }
}
