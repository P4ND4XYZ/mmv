fn main() {
    println!("cargo:rerun-if-changed=assets/mmv.rc");
    println!("cargo:rerun-if-changed=assets/mmv.ico");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // GPUI's Windows backend loads icon resource 1 from the executable.
        // Keep this resource on mmv only; test binaries need no application icon.
        embed_resource::compile_for("assets/mmv.rc", ["mmv"], embed_resource::NONE)
            .manifest_required()
            .expect(
                "failed to embed the mmv Windows icon (Windows SDK resource compiler required)",
            );
    }
}
