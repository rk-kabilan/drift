fn main() {
    println!("cargo:rerun-if-changed=assets/app-icon/windows/drift.rc");
    println!("cargo:rerun-if-changed=assets/app-icon/windows/drift.ico");
    // Embed the app icon into drift.exe. Does nothing when the target isn't Windows.
    embed_resource::compile("assets/app-icon/windows/drift.rc", embed_resource::NONE)
        .manifest_optional()
        .unwrap();
}
