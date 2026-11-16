//! On Windows, puts the app icon into Vesper.exe (package/windows/vesper.rc). Verified by CI only.

fn main() {
    println!("cargo:rerun-if-changed=package/windows/vesper.rc");
    println!("cargo:rerun-if-changed=package/windows/vesper.ico");
    #[cfg(windows)]
    if let Err(e) = embed_resource::compile("package/windows/vesper.rc", embed_resource::NONE)
        .manifest_required()
    {
        panic!("the icon resource did not compile: {e}");
    }
}
