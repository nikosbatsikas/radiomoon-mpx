//! Windows: gives each program its own icon and version information (packaging/windows/*.rc).
fn main() {
    println!("cargo:rerun-if-changed=packaging/windows");
    println!("cargo:rerun-if-changed=packaging/transmitter.ico");
    println!("cargo:rerun-if-changed=packaging/receiver.ico");
    let v = std::env::var("CARGO_PKG_VERSION").unwrap_or_else(|_| "0.0.0".into());
    let mut n = v.split(['.', '-']).map(|x| x.parse::<u32>().unwrap_or(0));
    let (a, b, c) = (n.next().unwrap_or(0), n.next().unwrap_or(0), n.next().unwrap_or(0));
    let defs = vec![format!("VER_MAJOR={a}"), format!("VER_MINOR={b}"), format!("VER_PATCH={c}"), format!("VER_TEXT=\"{v}\"")];
    // the icons are found through the include path (the packaging folder)
    let packaging = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into())).join("packaging");
    let inc = [packaging.into_os_string()];
    for (rc, bin) in [("packaging/windows/transmitter.rc", "radiomoon-mpx-transmitter"), ("packaging/windows/receiver.rc", "radiomoon-mpx-receiver")] {
        // NotWindows on Linux / macOS: nothing to do there
        let _ = embed_resource::compile_for(rc, [bin], embed_resource::ParamsMacrosAndIncludeDirs(&defs, &inc)).manifest_optional();
    }
}
