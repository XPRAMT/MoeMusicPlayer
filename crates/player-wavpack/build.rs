//! Compiles the decoder half of the vendored libwavpack (BSD-3-Clause, `vendor/COPYING`) with `cc`: no cmake, no
//! bindgen, no assembly, so it builds the same way for Windows and Android.

fn main() {
    let sources = [
        "common_utils",
        "decorr_utils",
        "entropy_utils",
        "open_utils",
        "open_legacy",
        "read_words",
        "tag_utils",
        "tags",
        "unpack",
        "unpack_dsd",
        "unpack_floats",
        "unpack_seek",
        "unpack_utils",
    ];
    let mut build = cc::Build::new();
    build
        .include("vendor/include")
        .include("vendor/src")
        .define("ENABLE_DSD", None)
        .warnings(false);
    if build.get_compiler().is_like_msvc() {
        build.define("_CRT_SECURE_NO_WARNINGS", None);
    } else {
        build.define("HAVE___BUILTIN_CLZ", None);
    }
    for source in sources {
        let path = format!("vendor/src/{source}.c");
        println!("cargo:rerun-if-changed={path}");
        build.file(path);
    }
    build.compile("wavpack");
}
