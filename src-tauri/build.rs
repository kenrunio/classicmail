fn main() {
    // Embed the Google OAuth client (Desktop app JSON) if `google-client.json` sits in the project
    // root, so a shipped installer works without importing the file on each computer.
    let path = std::path::Path::new("../google-client.json");
    println!("cargo:rerun-if-changed=../google-client.json");
    if let Ok(text) = std::fs::read_to_string(path) {
        let one_line = text.replace(['\r', '\n'], " ");
        println!("cargo:rustc-env=CLASSICMAIL_GOOGLE_CLIENT_JSON={one_line}");
    }

    // Our own Windows manifest (Common Controls v6 + DPI awareness), linked into EVERY artifact —
    // including test executables, which otherwise fail to start with 0xc0000139 because the
    // dialog/tray code imports comctl32 v6 entry points.
    let windows_msvc =
        std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    let attrs = if windows_msvc {
        let manifest = std::env::current_dir().unwrap().join("windows-app-manifest.xml");
        println!("cargo:rerun-if-changed=windows-app-manifest.xml");
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
        tauri_build::Attributes::new().windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest())
    } else {
        tauri_build::Attributes::new()
    };
    if windows_msvc {
        build_mapi_dlls();
    }
    tauri_build::try_build(attrs).expect("tauri build script");
}

/// The Simple MAPI DLL (crate `mapi/`) that makes ClassicMail the mail client Office's "Email"
/// button calls, built for 64- and 32-bit Office and embedded in the app (`mail::mapi`).
fn build_mapi_dlls() {
    for p in ["mapi/Cargo.toml", "mapi/Cargo.lock", "mapi/src"] {
        println!("cargo:rerun-if-changed={p}");
    }
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let target_dir = out.join("mapi-target");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    for (target, var) in [("x86_64-pc-windows-msvc", "CLASSICMAIL_MAPI_X64"), ("i686-pc-windows-msvc", "CLASSICMAIL_MAPI_X86")] {
        let mut cmd = std::process::Command::new(&cargo);
        cmd.args(["build", "--release", "--manifest-path", "mapi/Cargo.toml", "--target", target, "--target-dir"]).arg(&target_dir);
        // not this build's settings: its flags, jobserver and target dir
        for (k, _) in std::env::vars() {
            if (k.starts_with("CARGO_") && k != "CARGO_HOME") || k == "RUSTFLAGS" || k == "RUSTC_WRAPPER" {
                cmd.env_remove(k);
            }
        }
        let status = cmd.status().expect("run cargo for the MAPI DLL");
        assert!(status.success(), "building the MAPI DLL for {target} failed (32-bit needs: rustup target add i686-pc-windows-msvc)");
        let dll = target_dir.join(target).join("release").join("classicmail_mapi.dll");
        println!("cargo:rustc-env={var}={}", dll.display());
    }
}
