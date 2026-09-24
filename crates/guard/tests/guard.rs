use guard::{GuardState, Platform, RunningApp, detect};

fn mac(bundle_id: &str) -> RunningApp {
    RunningApp {
        bundle_id: Some(bundle_id.to_string()),
        exe_name: None,
    }
}

fn win(exe_name: &str) -> RunningApp {
    RunningApp {
        bundle_id: None,
        exe_name: Some(exe_name.to_string()),
    }
}

// ---------- detect: Mac (bundle id, exact) ----------

const MAC_IDS: &[(&str, &str)] = &[
    ("us.zoom.xos", "Zoom"),
    ("com.microsoft.teams", "Microsoft Teams"),
    ("com.microsoft.teams2", "Microsoft Teams"),
    ("com.hnc.Discord", "Discord"),
    ("com.hnc.DiscordPTB", "Discord"),
    ("com.hnc.DiscordCanary", "Discord"),
    ("com.obsproject.obs-studio", "OBS Studio"),
    ("com.apple.QuickTimePlayerX", "QuickTime Player"),
    ("com.apple.screenshot.launcher", "Screenshot"),
    ("com.apple.screencaptureui", "Screenshot"),
    ("com.loom.desktop", "Loom"),
    ("Cisco-Systems.Spark", "Webex"),
    ("com.webex.meetingmanager", "Webex"),
    ("com.tencent.meeting", "Tencent Meeting"),
    ("com.tencent.tencentmeeting", "Tencent Meeting"),
    ("com.skype.skype", "Skype"),
    ("pl.maketheweb.cleanshotx", "CleanShot X"),
    ("pl.maketheweb.cleanshotx-setapp", "CleanShot X"),
    ("com.TechSmith.Snagit2024", "Snagit"),
    ("com.TechSmith.Snagit2026", "Snagit"),
    ("com.timpler.screenstudio", "Screen Studio"),
    ("com.wulkano.kap", "Kap"),
];

#[test]
fn detect_mac_known_bundle_ids() {
    for (bundle_id, name) in MAC_IDS {
        assert_eq!(
            detect(Platform::Mac, &[mac(bundle_id)]),
            vec![*name],
            "bundle id {bundle_id}"
        );
    }
}

#[test]
fn detect_mac_ignores_unknown_and_near_misses() {
    let table: &[&str] = &[
        "com.tinyspeck.slackmacgap", // Slack is deliberately not included
        "com.google.Chrome",
        "US.ZOOM.XOS",       // exact match only
        "us.zoom.xos.extra", // no prefix matching
        "us.zoom",
        "",
    ];
    for bundle_id in table {
        assert!(
            detect(Platform::Mac, &[mac(bundle_id)]).is_empty(),
            "bundle id {bundle_id:?}"
        );
    }
}

#[test]
fn detect_mac_does_not_use_exe_names() {
    assert!(detect(Platform::Mac, &[win("Zoom.exe")]).is_empty());
}

// ---------- detect: Windows (exe name, case-insensitive exact) ----------

const WIN_EXES: &[(&str, &str)] = &[
    ("Zoom.exe", "Zoom"),
    ("CptHost.exe", "Zoom"),
    ("Teams.exe", "Microsoft Teams"),
    ("ms-teams.exe", "Microsoft Teams"),
    ("Discord.exe", "Discord"),
    ("DiscordPTB.exe", "Discord"),
    ("DiscordCanary.exe", "Discord"),
    ("obs64.exe", "OBS Studio"),
    ("obs32.exe", "OBS Studio"),
    ("Loom.exe", "Loom"),
    ("CiscoCollabHost.exe", "Webex"),
    ("webexmta.exe", "Webex"),
    ("atmgr.exe", "Webex"),
    ("WebexHost.exe", "Webex"),
    ("wemeetapp.exe", "Tencent Meeting"),
    ("voovmeetingapp.exe", "Tencent Meeting"),
    ("Skype.exe", "Skype"),
    ("SnippingTool.exe", "Snipping Tool"),
    ("ScreenClippingHost.exe", "Snipping Tool"),
    ("GameBar.exe", "Xbox Game Bar"),
    ("ShareX.exe", "ShareX"),
    ("Snagit32.exe", "Snagit"),
    ("SnagitCapture.exe", "Snagit"),
];

#[test]
fn detect_windows_known_exe_names() {
    for (exe, name) in WIN_EXES {
        assert_eq!(
            detect(Platform::Windows, &[win(exe)]),
            vec![*name],
            "exe {exe}"
        );
    }
}

#[test]
fn detect_windows_is_case_insensitive() {
    let table: &[&str] = &[
        "zoom.exe",
        "ZOOM.EXE",
        "ZoOm.ExE",
        "OBS64.EXE",
        "ms-TEAMS.exe",
    ];
    for exe in table {
        assert_eq!(detect(Platform::Windows, &[win(exe)]).len(), 1, "exe {exe}");
    }
}

#[test]
fn detect_windows_ignores_unknown_and_near_misses() {
    let table: &[&str] = &[
        "slack.exe", // Slack is deliberately not included
        "chrome.exe",
        "Zoom", // no extension: not an exact match
        "Zoom.exe.bak",
        "MyZoom.exe",
        " Zoom.exe",
        "",
    ];
    for exe in table {
        assert!(
            detect(Platform::Windows, &[win(exe)]).is_empty(),
            "exe {exe:?}"
        );
    }
}

#[test]
fn detect_windows_does_not_use_bundle_ids() {
    assert!(detect(Platform::Windows, &[mac("us.zoom.xos")]).is_empty());
}

// The two tables above are each checked against `detect`; together they must cover every app
// the crate docs require.
#[test]
fn detect_every_required_app_exists_on_some_platform() {
    for required in [
        "Zoom",
        "Microsoft Teams",
        "Discord",
        "OBS Studio",
        "QuickTime Player",
        "Screenshot",
        "Loom",
        "Webex",
        "Tencent Meeting",
        "Skype",
        "Snipping Tool",
        "Xbox Game Bar",
        "ShareX",
        "CleanShot X",
        "Snagit",
        "Screen Studio",
        "Kap",
    ] {
        assert!(
            MAC_IDS
                .iter()
                .chain(WIN_EXES)
                .any(|&(_, name)| name == required),
            "missing {required}"
        );
    }
}

#[test]
fn detect_returns_sorted_and_deduplicated_names() {
    let apps = [
        mac("us.zoom.xos"),
        mac("com.hnc.Discord"),
        mac("com.tinyspeck.slackmacgap"),
        mac("us.zoom.xos"),
        mac("com.hnc.DiscordPTB"),
        mac("com.apple.QuickTimePlayerX"),
    ];
    assert_eq!(
        detect(Platform::Mac, &apps),
        vec!["Discord", "QuickTime Player", "Zoom"]
    );
}

#[test]
fn detect_handles_empty_and_unknown_fields() {
    assert!(detect(Platform::Mac, &[]).is_empty());
    let blank = RunningApp {
        bundle_id: None,
        exe_name: None,
    };
    assert!(detect(Platform::Mac, std::slice::from_ref(&blank)).is_empty());
    assert!(detect(Platform::Windows, &[blank]).is_empty());
}

// ---------- GuardState ----------

#[test]
fn shield_is_down_with_nothing_detected() {
    let mut state = GuardState::new();
    assert!(!state.update(&[]));
    assert!(state.shield_apps().is_empty());
}

#[test]
fn shield_goes_up_while_a_known_app_runs_and_names_it() {
    let mut state = GuardState::new();
    assert!(state.update(&["Zoom"]));
    assert_eq!(
        state.shield_apps().into_iter().collect::<Vec<_>>(),
        vec!["Zoom"]
    );
}

#[test]
fn shield_goes_down_when_the_app_quits() {
    let mut state = GuardState::new();
    state.update(&["Zoom"]);
    assert!(!state.update(&[]));
    assert!(state.shield_apps().is_empty());
}

#[test]
fn dismiss_lowers_the_shield_while_the_same_apps_run() {
    let mut state = GuardState::new();
    state.update(&["Discord"]);
    state.dismiss(&["Discord"]);
    assert!(state.shield_apps().is_empty());
    assert!(!state.update(&["Discord"]));
    assert!(state.shield_apps().is_empty());
}

#[test]
fn dismissal_survives_a_subset_of_the_dismissed_apps() {
    let mut state = GuardState::new();
    state.update(&["Discord", "Zoom"]);
    state.dismiss(&["Discord", "Zoom"]);
    assert!(!state.update(&["Discord"]));
    assert!(!state.update(&[]));
    // An app that was in the set when dismissed coming back doesn't raise it.
    assert!(!state.update(&["Discord", "Zoom"]));
}

#[test]
fn a_new_app_raises_the_shield_again_and_names_all_running_apps() {
    let mut state = GuardState::new();
    state.update(&["Discord"]);
    state.dismiss(&["Discord"]);
    assert!(state.update(&["Discord", "Zoom"]));
    assert_eq!(
        state.shield_apps().into_iter().collect::<Vec<_>>(),
        vec!["Discord", "Zoom"]
    );
}

#[test]
fn after_a_new_app_ends_the_dismissal_the_old_apps_raise_the_shield() {
    let mut state = GuardState::new();
    state.update(&["Discord"]);
    state.dismiss(&["Discord"]);
    state.update(&["Discord", "Zoom"]);
    // Zoom quits; Discord alone now keeps the shield up because the dismissal ended.
    assert!(state.update(&["Discord"]));
}

#[test]
fn reset_forgets_the_dismissal() {
    let mut state = GuardState::new();
    state.update(&["Discord"]);
    state.dismiss(&["Discord"]);
    state.reset();
    assert!(state.update(&["Discord"]));
    assert_eq!(
        state.shield_apps().into_iter().collect::<Vec<_>>(),
        vec!["Discord"]
    );
}

#[test]
fn dismissing_with_nothing_running_means_any_app_raises_the_shield() {
    let mut state = GuardState::new();
    state.dismiss(&[]);
    assert!(state.update(&["Loom"]));
}
