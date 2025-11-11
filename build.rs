fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    let root = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let lib_dir = std::path::Path::new(&root).join("lib");
    let out_dir = std::env::var("OUT_DIR").unwrap();
    let out_dir = std::path::Path::new(&out_dir);
    for lib in std::fs::read_dir(lib_dir).unwrap() {
        let lib_path = lib.unwrap();
        std::fs::copy(lib_path.path(), out_dir.join(lib_path.file_name())).unwrap();
    }

    println!("cargo:rustc-link-search=native={}", out_dir.display());

    println!("cargo:rustc-link-lib=dylib=dokan2");
    println!("cargo:rustc-link-lib=dylib=dokanfuse2");
    println!("cargo:rustc-link-lib=dylib=imobiledevice-1.0");
    println!("cargo:rustc-link-lib=dylib=imobiledevice-glue-1.0");
    println!("cargo:rustc-link-lib=dylib=usbmuxd-2.0");
}
