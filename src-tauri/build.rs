// The app manifest lists every app command, so Tauri generates an `allow-<command>` permission
// for each and the webview can call only the ones `capabilities/main.json` grants (all of these,
// and nothing else). Keep this list in step with `src/api.ts` and `commands.rs`.
const COMMANDS: &[&str] = &[
    "app_status",
    "activity",
    "suggest_passphrase",
    "assess_password",
    "create_vault",
    "confirm_recovery_kit",
    "unlock",
    "set_new_password",
    "lock",
    "list_notes",
    "get_note",
    "create_note",
    "update_note",
    "trash_note",
    "restore_note",
    "delete_forever",
    "empty_trash",
    "set_favorite",
    "append_text",
    "show_field",
    "show_wallet_field",
    "show_and_copy_wallet_field",
    "hide_field",
    "extend_reveal",
    "copy_field",
    "copy_visible_field",
    "copy_field_line",
    "copy_field_env",
    "seed_wordlist",
    "check_seed",
    "check_private_key",
    "pick_backup_destination",
    "write_backup",
    "pick_backup_to_open",
    "inspect_backup",
    "replace_with_backup",
    "discard_inspection",
    "forget_backup_destination",
    "list_safety_copies",
    "inspect_safety_copy",
    "restore_safety_copy",
    "change_password",
    "set_settings",
    "mark_seed_explainer_seen",
    "vault_location",
    "dismiss_sharing_guard",
    // The optional Agent Companion (ADR-0009): status, start and stop, plus native owner setup.
    "companion_status",
    "companion_start",
    "companion_stop",
    "companion_setup",
];

fn main() {
    println!("cargo:rerun-if-changed=native/biometry.swift");
    println!("cargo:rerun-if-changed=native/companion/protocol.swift");
    println!("cargo:rerun-if-changed=native/companion/client.swift");
    #[cfg(target_os = "macos")]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        build_biometry();
        build_companion_client();
    }
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri-build");
}

#[cfg(target_os = "macos")]
fn build_biometry() {
    use std::{path::PathBuf, process::Command};
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x86_64",
        _ => panic!("unsupported macOS architecture"),
    };
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let object = out.join("biometry.o");
    let archive = out.join("libencrypted_note_biometry.a");
    let status = Command::new("xcrun")
        .args([
            "swiftc",
            "-parse-as-library",
            "-emit-object",
            "-O",
            "-target",
        ])
        .arg(format!("{arch}-apple-macos13.0"))
        .args(["-module-name", "EncryptedNoteBiometry", "-o"])
        .arg(&object)
        .arg("native/biometry.swift")
        .status()
        .expect("run Swift compiler");
    assert!(status.success(), "compile local biometric bridge");
    let status = Command::new("xcrun")
        .args(["ar", "crs"])
        .arg(&archive)
        .arg(&object)
        .status()
        .expect("run archive tool");
    assert!(status.success(), "archive local biometric bridge");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=encrypted_note_biometry");
    for framework in ["CryptoKit", "Security", "LocalAuthentication", "Foundation"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
    swift_rs::SwiftLinker::new("13.0").link();
}

/// The app side of the Agent Companion (ADR-0009): forwards status/start/stop/setup to the
/// bundled helper over XPC and copies back four numbers. The helper itself is built and signed
/// separately by scripts/package-companion.sh; it is never linked into this process.
#[cfg(target_os = "macos")]
fn build_companion_client() {
    use std::{path::PathBuf, process::Command};
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x86_64",
        _ => panic!("unsupported macOS architecture"),
    };
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let object = out.join("companion_client.o");
    let archive = out.join("libencrypted_note_companion.a");
    let status = Command::new("xcrun")
        .args([
            "swiftc",
            "-parse-as-library",
            "-wmo",
            "-emit-object",
            "-O",
            "-target",
        ])
        .arg(format!("{arch}-apple-macos13.0"))
        .args(["-module-name", "EncryptedNoteCompanion", "-o"])
        .arg(&object)
        .args([
            "native/companion/protocol.swift",
            "native/companion/client.swift",
        ])
        .status()
        .expect("run Swift compiler");
    assert!(status.success(), "compile Agent Companion client");
    let status = Command::new("xcrun")
        .args(["ar", "crs"])
        .arg(&archive)
        .arg(&object)
        .status()
        .expect("run archive tool");
    assert!(status.success(), "archive Agent Companion client");
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=encrypted_note_companion");
    println!("cargo:rustc-link-lib=framework=Foundation");
}
