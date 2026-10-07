fn main() {
    // The Windows exe and window icon are compiled into the resource file when
    // this script runs. tauri-build watches tauri.conf.json, not icons/icon.ico,
    // so an icon-only change would otherwise keep the previous embedded icon.
    println!("cargo:rerun-if-changed=icons/icon.ico");
    tauri_build::build()
}
