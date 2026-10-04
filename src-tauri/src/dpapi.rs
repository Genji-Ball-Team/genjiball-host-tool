//! Windows DPAPI: encrypts bytes with a key tied to the Windows user account, so only that user
//! on that PC can read them back. The fallback for the host token when Credential Manager won't
//! take it (`credentials.rs`).

/// Mixed into the key, so the blob is only readable by code that knows it (not a secret).
const ENTROPY: &[u8] = b"genjiball-host-tool";

#[cfg(windows)]
mod imp {
    use std::{ptr, slice};

    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };

    fn blob(data: &[u8]) -> CRYPT_INTEGER_BLOB {
        CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        }
    }

    /// Copies the output out of the buffer Windows allocated, then frees it.
    unsafe fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
        let bytes = slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData.cast());
        bytes
    }

    pub fn protect(data: &[u8]) -> Result<Vec<u8>, String> {
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: the input blobs point at live slices for the call; `out` is freed by `take`.
        let ok = unsafe {
            CryptProtectData(
                &blob(data),
                ptr::null(),
                &blob(super::ENTROPY),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            return Err(format!(
                "Windows couldn't encrypt it: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(unsafe { take(out) })
    }

    pub fn unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: as in `protect`.
        let ok = unsafe {
            CryptUnprotectData(
                &blob(data),
                ptr::null_mut(),
                &blob(super::ENTROPY),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            return Err(format!(
                "Windows couldn't decrypt it: {}",
                std::io::Error::last_os_error()
            ));
        }
        Ok(unsafe { take(out) })
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn protect(_: &[u8]) -> Result<Vec<u8>, String> {
        Err("Only Windows can encrypt the token outside the credential store".into())
    }

    pub fn unprotect(_: &[u8]) -> Result<Vec<u8>, String> {
        Err("Only Windows can decrypt the token outside the credential store".into())
    }
}

pub use imp::{protect, unprotect};

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_hides_the_text() {
        let secret = b"e2e-good-token";
        let sealed = protect(secret).unwrap();
        assert!(!sealed.windows(secret.len()).any(|w| w == secret));
        assert_eq!(unprotect(&sealed).unwrap(), secret);
    }

    #[test]
    fn refuses_tampered_data() {
        let mut sealed = protect(b"token").unwrap();
        let last = sealed.len() - 1;
        sealed[last] ^= 0xff;
        assert!(unprotect(&sealed).is_err());
    }
}
