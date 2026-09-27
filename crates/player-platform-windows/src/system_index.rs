use std::path::Path;

use std::os::windows::ffi::OsStrExt;
use windows::{
    core::{w, PCWSTR},
    Win32::System::{
        Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_LOCAL_SERVER,
            COINIT_MULTITHREADED,
        },
        Search::{
            CSearchManager, CatalogStatus, ISearchManager, CATALOG_PAUSED_REASON_NONE,
            CATALOG_STATUS_IDLE,
        },
        Services::{
            CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatus, SC_HANDLE,
            SC_MANAGER_CONNECT, SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_STATUS,
        },
    },
};

/// Inspect Windows Search health and the configured root scope, but never treat its result set as
/// a complete source snapshot. Official API limits that prevent completeness proof are described
/// in `SYSTEM_INDEX.md`.
pub(crate) fn diagnostic(root: &Path) -> String {
    match probe(root) {
        Ok(details) => format!(
            "SystemIndex diagnostic: {details}. Windows Search does not expose a root-completeness or freshness proof (including per-file FANCI and crawler omissions), so its result set is not used; filesystem fallback supplies all records and the deletion reconciliation set."
        ),
        Err(error) => format!(
            "SystemIndex diagnostic unavailable: {error}. Its result set is not used; filesystem fallback supplies all records and the deletion reconciliation set."
        ),
    }
}

fn probe(root: &Path) -> Result<String, String> {
    let service_state = windows_search_service_state()?;
    if service_state != SERVICE_RUNNING.0 {
        return Ok(format!(
            "Windows Search service WSearch is not running (state {service_state})"
        ));
    }

    let scope_url_units = scope_url(root)?;
    let _apartment = ComApartment::initialize()?;
    let manager: ISearchManager = unsafe {
        CoCreateInstance(&CSearchManager, None, CLSCTX_LOCAL_SERVER)
            .map_err(|error| format!("could not connect to Windows Search: {error}"))?
    };
    let catalog = unsafe { manager.GetCatalog(w!("SystemIndex")) }
        .map_err(|error| format!("could not open SystemIndex: {error}"))?;
    let scope_manager = unsafe { catalog.GetCrawlScopeManager() }
        .map_err(|error| format!("could not inspect SystemIndex crawl scope: {error}"))?;

    let mut status: CatalogStatus = CATALOG_STATUS_IDLE;
    let mut paused_reason = CATALOG_PAUSED_REASON_NONE;
    unsafe { catalog.GetCatalogStatus(&mut status, &mut paused_reason) }
        .map_err(|error| format!("could not read SystemIndex catalog status: {error}"))?;

    let scope_url = PCWSTR::from_raw(scope_url_units.as_ptr());
    let included = unsafe { scope_manager.IncludedInCrawlScope(scope_url) }
        .map_err(|error| format!("could not check whether root is in crawl scope: {error}"))?;
    let has_child_rule = unsafe { scope_manager.HasChildScopeRule(scope_url) }
        .map_err(|error| format!("could not check child crawl-scope rules: {error}"))?;

    Ok(format!(
        "WSearch is running; root included by crawl-scope rules: {}; descendant scope rules present: {}; global catalog status: {} (paused reason {})",
        included.as_bool(),
        has_child_rule.as_bool(),
        status.0,
        paused_reason.0,
    ))
}

fn windows_search_service_state() -> Result<u32, String> {
    let manager = unsafe {
        OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_CONNECT)
            .map_err(|error| format!("could not inspect Windows services: {error}"))?
    };
    let _manager = ServiceHandle(manager);
    let service = unsafe {
        OpenServiceW(manager, w!("WSearch"), SERVICE_QUERY_STATUS)
            .map_err(|error| format!("could not open Windows Search service WSearch: {error}"))?
    };
    let _service = ServiceHandle(service);
    let mut status = SERVICE_STATUS::default();
    unsafe { QueryServiceStatus(service, &mut status) }
        .map_err(|error| format!("could not query Windows Search service WSearch: {error}"))?;
    Ok(status.dwCurrentState.0)
}

struct ServiceHandle(SC_HANDLE);

impl Drop for ServiceHandle {
    fn drop(&mut self) {
        let _ = unsafe { CloseServiceHandle(self.0) };
    }
}

struct ComApartment;

impl ComApartment {
    fn initialize() -> Result<Self, String> {
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }
            .ok()
            .map_err(|error| format!("could not initialize COM for Windows Search: {error}"))?;
        Ok(Self)
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

/// Convert a local absolute Windows path to a file URL without lossy `Path` conversion. UNC roots
/// and invalid UTF-16 are left to the filesystem fallback because they cannot be safely mapped by
/// this diagnostic probe.
fn scope_url(path: &Path) -> Result<Vec<u16>, String> {
    let mut path_units = path.as_os_str().encode_wide().collect::<Vec<_>>();
    const VERBATIM_PREFIX: &[u16] = &[b'\\' as u16, b'\\' as u16, b'?' as u16, b'\\' as u16];
    const VERBATIM_UNC: &[u16] = &[
        b'\\' as u16,
        b'\\' as u16,
        b'?' as u16,
        b'\\' as u16,
        b'U' as u16,
        b'N' as u16,
        b'C' as u16,
        b'\\' as u16,
    ];

    let has_verbatim_prefix = path_units.starts_with(VERBATIM_PREFIX);
    if starts_with_ascii_case_insensitive(&path_units, VERBATIM_UNC)
        || (!has_verbatim_prefix && path_units.starts_with(&[b'\\' as u16, b'\\' as u16]))
    {
        return Err("UNC paths are not evaluated through SystemIndex".to_owned());
    }
    if path_units.starts_with(VERBATIM_PREFIX) {
        path_units.drain(..VERBATIM_PREFIX.len());
    }

    let path_text = String::from_utf16(&path_units).map_err(|_| {
        "root path contains invalid UTF-16; no lossy conversion was attempted".to_owned()
    })?;
    let drive = path_text.as_bytes().get(0..2);
    if !drive.is_some_and(|value| value[0].is_ascii_alphabetic() && value[1] == b':') {
        return Err("root is not a local drive path".to_owned());
    }

    let normalized = path_text.replace('\\', "/");
    let rest = normalized[2..].trim_start_matches('/');
    let mut url = String::from("file:///");
    url.push_str(&normalized[..2]);
    url.push('/');
    for character in rest.chars() {
        if character == '/' {
            url.push('/');
        } else {
            push_uri_component(&mut url, character);
        }
    }
    if !url.ends_with('/') {
        url.push('/');
    }

    let mut url_units = url.encode_utf16().collect::<Vec<_>>();
    url_units.push(0);
    Ok(url_units)
}

fn starts_with_ascii_case_insensitive(value: &[u16], prefix: &[u16]) -> bool {
    value.get(..prefix.len()).is_some_and(|head| {
        head.iter().zip(prefix).all(|(left, right)| {
            let left = if (b'a' as u16..=b'z' as u16).contains(left) {
                left - 32
            } else {
                *left
            };
            let right = if (b'a' as u16..=b'z' as u16).contains(right) {
                right - 32
            } else {
                *right
            };
            left == right
        })
    })
}

fn push_uri_component(url: &mut String, character: char) {
    if character.is_ascii_alphanumeric() || matches!(character, '-' | '.' | '_' | '~') {
        url.push(character);
        return;
    }
    for byte in character.encode_utf8(&mut [0; 4]).as_bytes() {
        use std::fmt::Write;
        let _ = write!(url, "%{byte:02X}");
    }
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf};

    use super::{diagnostic, scope_url};

    #[test]
    fn scope_url_encodes_unicode_and_reserved_path_characters() {
        let ordinary = PathBuf::from(r"C:\音樂\東京🌸 #mix");
        let url_units = scope_url(ordinary.as_path()).unwrap();
        let extended = PathBuf::from(r"\\?\C:\音樂\東京🌸 #mix");
        assert_eq!(scope_url(extended.as_path()).unwrap(), url_units);
        let url = String::from_utf16(&url_units).unwrap();
        assert_eq!(
            url.trim_end_matches('\0'),
            "file:///C:/%E9%9F%B3%E6%A8%82/%E6%9D%B1%E4%BA%AC%F0%9F%8C%B8%20%23mix/"
        );
    }

    #[test]
    fn scope_url_rejects_unc_and_invalid_utf16_without_lossy_conversion() {
        assert!(scope_url(PathBuf::from(r"\\server\share\music").as_path()).is_err());

        let invalid = OsString::from_wide(&[b'C' as u16, b':' as u16, b'\\' as u16, 0xD800]);
        assert!(scope_url(PathBuf::from(invalid).as_path()).is_err());
    }

    #[test]
    fn system_index_diagnostic_never_promises_complete_results() {
        let diagnostic = diagnostic(std::env::temp_dir().as_path());
        eprintln!("{diagnostic}");
        assert!(diagnostic.contains("filesystem fallback supplies all records"));
        assert!(diagnostic.contains("deletion reconciliation set"));
    }
}
