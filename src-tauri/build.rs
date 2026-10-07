use std::fs;

fn main() {
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
        return format!("{}.{}.{}", time.year % 100, time.month, time.day);
    }
    #[cfg(not(windows))]
    {
        let _ = ();
        "0.0.0".to_owned()
    }
}
