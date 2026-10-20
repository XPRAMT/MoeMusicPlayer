//! Our own credential encryption for remote sources (ADR 0026): no operating-system credential store.
//!
//! Two kinds of sealed text, both XChaCha20-Poly1305 (a fresh random 192-bit nonce per message, the scope bound in as
//! associated data so a sealed password cannot be moved to another account):
//!
//! * a **device-sealed secret** (`v1.<nonce>.<ciphertext>`), keyed by a random 256-bit **device key** kept in a file
//!   beside the settings. It is what `settings.json` stores for an account's password: the file never holds plaintext,
//!   and copying `settings.json` alone (a backup, a screenshot, a support request) reveals nothing. The device key
//!   file is the root of trust of this machine, so someone who copies the whole data folder can read the passwords;
//!   this is stated in the user interface;
//! * a **passphrase-sealed bundle** (`p1.<salt>.<m>.<t>.<p>.<nonce>.<ciphertext>`), keyed by Argon2id over a
//!   passphrase the user types. It is what leaves the machine: the account export file and the accounts inside a full
//!   backup, so they can be restored on another device that has a different device key.

use std::fmt;
use std::io::Write;
use std::path::Path;

use argon2::{Algorithm, Argon2, Params, Version};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use chacha20poly1305::aead::{Aead, KeyInit, OsRng, Payload};
use chacha20poly1305::{AeadCore, XChaCha20Poly1305, XNonce};
use zeroize::{Zeroize, Zeroizing};

const DEVICE_KEY_HEADER: &str = "MOE-REMOTE-KEY-1";
const KEY_LEN: usize = 32;
const NONCE_LEN: usize = 24;
const SALT_LEN: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretError {
    /// The device key file could not be read, written or is damaged.
    Key(String),
    /// The sealed text is not in a known format.
    Format(String),
    /// Authentication failed: the wrong key or passphrase, or the data was changed.
    Decrypt,
}

impl fmt::Display for SecretError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key(message) => write!(f, "裝置金鑰檔案有問題：{message}"),
            Self::Format(message) => write!(f, "加密資料的格式不正確：{message}"),
            Self::Decrypt => write!(f, "無法解密：密碼錯誤，或資料已被更動"),
        }
    }
}

impl std::error::Error for SecretError {}

/// The random key of this machine; wiped from memory when dropped.
pub struct DeviceKey(Zeroizing<[u8; KEY_LEN]>);

impl DeviceKey {
    pub fn generate() -> Self {
        let mut key = Zeroizing::new([0_u8; KEY_LEN]);
        key.copy_from_slice(&XChaCha20Poly1305::generate_key(&mut OsRng));
        Self(key)
    }

    /// Read the key file, creating it (never overwriting an existing one) when it is not there yet.
    pub fn load_or_create(path: &Path) -> Result<Self, SecretError> {
        match std::fs::read_to_string(path) {
            Ok(text) => return Self::parse(&text),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(SecretError::Key(error.to_string())),
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| SecretError::Key(error.to_string()))?;
        }
        let key = Self::generate();
        let text = format!(
            "{DEVICE_KEY_HEADER}\n{}\n",
            URL_SAFE_NO_PAD.encode(key.0.as_slice())
        );
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
        {
            Ok(mut file) => {
                file.write_all(text.as_bytes())
                    .and_then(|()| file.sync_all())
                    .map_err(|error| SecretError::Key(error.to_string()))?;
                Ok(key)
            }
            // Another thread or process created it between the read and the create: use theirs.
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Self::parse(
                &std::fs::read_to_string(path).map_err(|e| SecretError::Key(e.to_string()))?,
            ),
            Err(error) => Err(SecretError::Key(error.to_string())),
        }
    }

    fn parse(text: &str) -> Result<Self, SecretError> {
        let mut lines = text.lines();
        if lines.next().map(str::trim) != Some(DEVICE_KEY_HEADER) {
            return Err(SecretError::Key("不是本程式的金鑰檔".to_owned()));
        }
        let encoded = lines.next().unwrap_or_default().trim();
        let bytes = Zeroizing::new(
            URL_SAFE_NO_PAD
                .decode(encoded)
                .map_err(|_| SecretError::Key("金鑰內容無法解碼".to_owned()))?,
        );
        if bytes.len() != KEY_LEN {
            return Err(SecretError::Key("金鑰長度不正確".to_owned()));
        }
        let mut key = Zeroizing::new([0_u8; KEY_LEN]);
        key.copy_from_slice(&bytes);
        Ok(Self(key))
    }
}

fn aead_seal(key: &[u8], aad: &[u8], plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>), SecretError> {
    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| SecretError::Format("金鑰長度".to_owned()))?;
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .map_err(|_| SecretError::Decrypt)?;
    Ok((nonce.to_vec(), ciphertext))
}

fn aead_open(
    key: &[u8],
    aad: &[u8],
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<Zeroizing<Vec<u8>>, SecretError> {
    if nonce.len() != NONCE_LEN {
        return Err(SecretError::Format("nonce 長度".to_owned()));
    }
    let cipher = XChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| SecretError::Format("金鑰長度".to_owned()))?;
    cipher
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .map(Zeroizing::new)
        .map_err(|_| SecretError::Decrypt)
}

/// `bytes` random bytes from the operating system's generator, as lower-case hex (ids, not secrets to keep).
pub fn random_hex(bytes: usize) -> String {
    use chacha20poly1305::aead::rand_core::RngCore as _;
    let mut buffer = vec![0_u8; bytes];
    OsRng.fill_bytes(&mut buffer);
    buffer.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn aad(scope: &str) -> Vec<u8> {
    format!("moe-remote:{scope}").into_bytes()
}

fn decode(part: &str, what: &str) -> Result<Vec<u8>, SecretError> {
    URL_SAFE_NO_PAD
        .decode(part)
        .map_err(|_| SecretError::Format(format!("{what} 無法解碼")))
}

/// Seal `plaintext` under the device key. `scope` (for example `account:<source id>:password`) is authenticated with
/// the data, so the sealed text only opens under the same scope.
pub fn seal(key: &DeviceKey, scope: &str, plaintext: &str) -> Result<String, SecretError> {
    let (nonce, ciphertext) = aead_seal(key.0.as_slice(), &aad(scope), plaintext.as_bytes())?;
    Ok(format!(
        "v1.{}.{}",
        URL_SAFE_NO_PAD.encode(nonce),
        URL_SAFE_NO_PAD.encode(ciphertext)
    ))
}

pub fn open(key: &DeviceKey, scope: &str, sealed: &str) -> Result<Zeroizing<String>, SecretError> {
    let mut parts = sealed.split('.');
    if parts.next() != Some("v1") {
        return Err(SecretError::Format("不是裝置加密的內容".to_owned()));
    }
    let nonce = decode(parts.next().unwrap_or_default(), "nonce")?;
    let ciphertext = decode(parts.next().unwrap_or_default(), "密文")?;
    if parts.next().is_some() {
        return Err(SecretError::Format("多餘的欄位".to_owned()));
    }
    let plaintext = aead_open(key.0.as_slice(), &aad(scope), &nonce, &ciphertext)?;
    String::from_utf8(plaintext.to_vec())
        .map(Zeroizing::new)
        .map_err(|_| SecretError::Format("內容不是文字".to_owned()))
}

/// Argon2id cost of a passphrase-sealed bundle. The defaults take roughly a quarter of a second on a phone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KdfCost {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl Default for KdfCost {
    fn default() -> Self {
        Self {
            memory_kib: 64 * 1024,
            iterations: 3,
            parallelism: 1,
        }
    }
}

/// The largest cost accepted when opening a bundle, so a crafted file cannot make the app allocate gigabytes.
const MAX_KDF_MEMORY_KIB: u32 = 512 * 1024;
const MAX_KDF_ITERATIONS: u32 = 10;
const MAX_KDF_PARALLELISM: u32 = 8;

fn derive(
    passphrase: &str,
    salt: &[u8],
    cost: KdfCost,
) -> Result<Zeroizing<[u8; KEY_LEN]>, SecretError> {
    let params = Params::new(
        cost.memory_kib,
        cost.iterations,
        cost.parallelism,
        Some(KEY_LEN),
    )
    .map_err(|error| SecretError::Format(error.to_string()))?;
    let mut key = Zeroizing::new([0_u8; KEY_LEN]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(passphrase.as_bytes(), salt, key.as_mut_slice())
        .map_err(|error| SecretError::Format(error.to_string()))?;
    Ok(key)
}

/// Seal `plaintext` under a passphrase (an account export). `purpose` names what the bundle is and is authenticated.
pub fn seal_with_passphrase(
    passphrase: &str,
    purpose: &str,
    plaintext: &[u8],
    cost: KdfCost,
) -> Result<String, SecretError> {
    if passphrase.is_empty() {
        return Err(SecretError::Format("密碼不可空白".to_owned()));
    }
    let mut salt = [0_u8; SALT_LEN];
    salt.copy_from_slice(&XChaCha20Poly1305::generate_key(&mut OsRng)[..SALT_LEN]);
    let mut key = derive(passphrase, &salt, cost)?;
    let (nonce, ciphertext) = aead_seal(key.as_slice(), &aad(purpose), plaintext)?;
    key.zeroize();
    Ok(format!(
        "p1.{}.{}.{}.{}.{}.{}",
        URL_SAFE_NO_PAD.encode(salt),
        cost.memory_kib,
        cost.iterations,
        cost.parallelism,
        URL_SAFE_NO_PAD.encode(nonce),
        URL_SAFE_NO_PAD.encode(ciphertext)
    ))
}

pub fn open_with_passphrase(
    passphrase: &str,
    purpose: &str,
    sealed: &str,
) -> Result<Zeroizing<Vec<u8>>, SecretError> {
    let mut parts = sealed.split('.');
    if parts.next() != Some("p1") {
        return Err(SecretError::Format("不是密碼加密的內容".to_owned()));
    }
    let salt = decode(parts.next().unwrap_or_default(), "salt")?;
    let number = |part: Option<&str>| -> Result<u32, SecretError> {
        part.and_then(|text| text.parse().ok())
            .ok_or_else(|| SecretError::Format("金鑰推導參數".to_owned()))
    };
    let cost = KdfCost {
        memory_kib: number(parts.next())?,
        iterations: number(parts.next())?,
        parallelism: number(parts.next())?,
    };
    if cost.memory_kib > MAX_KDF_MEMORY_KIB
        || cost.iterations > MAX_KDF_ITERATIONS
        || cost.parallelism > MAX_KDF_PARALLELISM
        || salt.len() < 8
    {
        return Err(SecretError::Format("金鑰推導參數超出允許範圍".to_owned()));
    }
    let nonce = decode(parts.next().unwrap_or_default(), "nonce")?;
    let ciphertext = decode(parts.next().unwrap_or_default(), "密文")?;
    if parts.next().is_some() {
        return Err(SecretError::Format("多餘的欄位".to_owned()));
    }
    let key = derive(passphrase, &salt, cost)?;
    aead_open(key.as_slice(), &aad(purpose), &nonce, &ciphertext)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("moe-remote-secret-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fast() -> KdfCost {
        KdfCost {
            memory_kib: 64,
            iterations: 1,
            parallelism: 1,
        }
    }

    #[test]
    fn a_sealed_password_never_shows_the_plaintext_and_opens_under_its_scope_only() {
        let key = DeviceKey::generate();
        let sealed = seal(&key, "account:a1:password", "p@ss/wörd 密碼").unwrap();
        assert!(sealed.starts_with("v1."));
        assert!(!sealed.contains("p@ss") && !sealed.contains("密碼"));
        assert_eq!(
            &*open(&key, "account:a1:password", &sealed).unwrap(),
            "p@ss/wörd 密碼"
        );
        assert_eq!(
            open(&key, "account:a2:password", &sealed),
            Err(SecretError::Decrypt),
            "another account's scope"
        );
        assert_eq!(
            open(&DeviceKey::generate(), "account:a1:password", &sealed),
            Err(SecretError::Decrypt)
        );
    }

    #[test]
    fn every_seal_uses_a_fresh_nonce() {
        let key = DeviceKey::generate();
        let a = seal(&key, "s", "same").unwrap();
        let b = seal(&key, "s", "same").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn tampered_or_malformed_sealed_text_is_refused() {
        let key = DeviceKey::generate();
        let sealed = seal(&key, "s", "secret").unwrap();
        let mut bytes = sealed.clone().into_bytes();
        let last = bytes.len() - 3;
        bytes[last] = if bytes[last] == b'A' { b'B' } else { b'A' };
        assert_eq!(
            open(&key, "s", &String::from_utf8(bytes).unwrap()),
            Err(SecretError::Decrypt)
        );
        for bad in ["", "v2.a.b", "v1.only", "v1.a.b.c", "plaintext password"] {
            assert!(
                matches!(open(&key, "s", bad), Err(SecretError::Format(_))),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn the_device_key_file_is_created_once_and_read_back() {
        let dir = temp_dir("key");
        let path = dir.join("remote-credentials.key");
        let first = DeviceKey::load_or_create(&path).unwrap();
        let sealed = seal(&first, "s", "hello").unwrap();
        let second = DeviceKey::load_or_create(&path).unwrap();
        assert_eq!(
            &*open(&second, "s", &sealed).unwrap(),
            "hello",
            "the same key on the next start"
        );
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(DEVICE_KEY_HEADER));
        std::fs::write(&path, "garbage").unwrap();
        assert!(
            matches!(DeviceKey::load_or_create(&path), Err(SecretError::Key(_))),
            "never overwritten, never guessed"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_passphrase_bundle_opens_with_the_passphrase_only() {
        let sealed =
            seal_with_passphrase("correct horse", "export", b"{\"accounts\":[]}", fast()).unwrap();
        assert!(sealed.starts_with("p1."));
        assert_eq!(
            &*open_with_passphrase("correct horse", "export", &sealed).unwrap(),
            b"{\"accounts\":[]}"
        );
        assert_eq!(
            open_with_passphrase("wrong", "export", &sealed),
            Err(SecretError::Decrypt)
        );
        assert_eq!(
            open_with_passphrase("correct horse", "backup", &sealed),
            Err(SecretError::Decrypt),
            "another purpose"
        );
        assert!(seal_with_passphrase("", "export", b"x", fast()).is_err());
    }

    #[test]
    fn a_bundle_cannot_demand_absurd_key_derivation_costs() {
        let sealed = seal_with_passphrase("pw", "export", b"x", fast()).unwrap();
        let mut parts: Vec<&str> = sealed.split('.').collect();
        parts[2] = "4000000";
        let crafted = parts.join(".");
        assert!(matches!(
            open_with_passphrase("pw", "export", &crafted),
            Err(SecretError::Format(_))
        ));
    }
}
