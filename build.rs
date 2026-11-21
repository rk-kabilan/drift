use std::path::PathBuf;

fn main() {
    const TEMPLATE: &str = "assets/app-icon/windows/drift.rc";
    println!("cargo:rerun-if-changed={TEMPLATE}");
    println!("cargo:rerun-if-changed=assets/app-icon/windows/drift.ico");

    // Fill the resource template with the icon's full path and the version from
    // Cargo.toml, so drift.exe's properties always show the current version.
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let icon = manifest.join("assets/app-icon/windows/drift.ico");
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let part = |name: &str| std::env::var(name).unwrap_or_else(|_| "0".into());
    let commas = format!(
        "{},{},{},0",
        part("CARGO_PKG_VERSION_MAJOR"),
        part("CARGO_PKG_VERSION_MINOR"),
        part("CARGO_PKG_VERSION_PATCH")
    );
    let rc = std::fs::read_to_string(manifest.join(TEMPLATE))
        .unwrap()
        .replace("@ICON@", &icon.display().to_string().replace('\\', "/"))
        .replace("@VERSION_COMMAS@", &commas)
        .replace("@VERSION@", &version);
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("drift.rc");
    std::fs::write(&out, rc).unwrap();

    // Embed the icon and version into drift.exe. Does nothing when the target isn't Windows.
    embed_resource::compile(&out, embed_resource::NONE)
        .manifest_optional()
        .unwrap();
}
