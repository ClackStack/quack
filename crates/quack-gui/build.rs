fn main() {
    // The Tauri codegen runs only for the desktop shell; without the feature this is a
    // no-op so default workspace builds never touch tauri-build.
    #[cfg(feature = "desktop")]
    tauri_build::build();
}
