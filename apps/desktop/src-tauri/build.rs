fn main() {
    // Tauri's default resource compiler links its manifest only to binaries.
    // Mock-runtime unit tests also link native dialogs, whose TaskDialogIndirect
    // import requires Common Controls v6 before the executable can load.
    // Embed the same dependency in every executable, with one manifest each.
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest()),
    )
    .expect("could not build Tauri resources");
    println!("cargo:rerun-if-changed=windows-manifest.xml");
    println!("cargo:rerun-if-changed=windows-manifest.rc");
    embed_resource::compile_for_everything("windows-manifest.rc", embed_resource::NONE)
        .manifest_required()
        .expect("could not embed the Windows application manifest");
}
