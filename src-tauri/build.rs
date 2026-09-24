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
    "show_field",
    "show_wallet_field",
    "hide_field",
    "copy_field",
    "seed_wordlist",
    "check_seed",
    "check_private_key",
    "pick_backup_destination",
    "write_backup",
    "pick_backup_to_open",
    "inspect_backup",
    "replace_with_backup",
    "list_safety_copies",
    "inspect_safety_copy",
    "restore_safety_copy",
    "change_password",
    "set_settings",
    "mark_seed_explainer_seen",
    "dismiss_sharing_guard",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri-build");
}
