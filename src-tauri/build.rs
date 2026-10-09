use std::fs;

fn main() {
    println!("cargo:rerun-if-changed=../.git/HEAD");
    println!("cargo:rerun-if-changed=../.git/refs/heads/master");
    println!("cargo:rerun-if-env-changed=MOE_BUILD_TIMESTAMP_UTC");
    let timestamp = std::env::var("MOE_BUILD_TIMESTAMP_UTC").unwrap_or_else(|_| utc_build_time());
    let commit = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        .unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=MOE_BUILD_TIMESTAMP_UTC={timestamp}");
    println!("cargo:rustc-env=MOE_GIT_COMMIT={commit}");
    // The Windows exe and window icon are compiled into the resource file when
    // this script runs. tauri-build watches tauri.conf.json, not icons/icon.ico,
    // so an icon-only change would otherwise keep the previous embedded icon.
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=build-version.txt");
    let version = fs::read_to_string("build-version.txt")
        .ok()
        .map(|text| text.trim().to_owned())
        .filter(|text| !text.is_empty())
        .unwrap_or_else(local_compile_version);
    println!("cargo:rustc-env=MOE_BUILD_VERSION={version}");
    tauri_build::build()
}

// Gregorian civil date conversion from UTC days since 1970; no local timezone
// or shell tools participate in update ordering.
fn utc_build_time() -> String {
    let elapsed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("build clock predates Unix epoch");
    let seconds = elapsed.as_secs();
    let days = (seconds / 86_400) as i64 + 719_468;
    let era = days / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        seconds / 3600 % 24,
        seconds / 60 % 60,
        seconds % 60,
        elapsed.subsec_millis()
    )
}

fn local_compile_version() -> String {
    #[cfg(windows)]
    {
        #[repr(C)]
        struct SystemTime {
            year: u16,
            month: u16,
            day_of_week: u16,
            day: u16,
            hour: u16,
            minute: u16,
            second: u16,
            milliseconds: u16,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetLocalTime(time: *mut SystemTime);
        }
        let mut time = SystemTime {
            year: 0,
            month: 0,
            day_of_week: 0,
            day: 0,
            hour: 0,
            minute: 0,
            second: 0,
            milliseconds: 0,
        };
        unsafe { GetLocalTime(&mut time) };
        format!("{}.{}.{}", time.year % 100, time.month, time.day)
    }
    #[cfg(not(windows))]
    {
        let _ = ();
        "0.0.0".to_owned()
    }
}
