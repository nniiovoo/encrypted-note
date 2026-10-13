//! Sharing Guard logic (ADR-0002, CONTEXT.md). Pure: the app lists running processes with an
//! OS adapter and asks this crate what they mean.
//!
//! macOS has no API that tells an app it is being captured, so this is a running-apps
//! heuristic: while any known sharing or recording app is running, the shield is up. The owner
//! may dismiss it ("I'm not sharing right now"). The dismissal covers the apps running at that
//! moment for as long as each keeps running: it ends when an app appears that it doesn't cover
//! (a new one, or a covered one that quit and came back), or when the Vault locks.
//!
//! The known-app table covers, per platform, at least: Zoom, Microsoft Teams (classic and new),
//! Discord, OBS Studio, QuickTime Player, macOS Screenshot (Cmd+Shift+5 UI), Loom, Webex,
//! Tencent Meeting / VooV, Skype, Snipping Tool, Xbox Game Bar, ShareX, CleanShot X, Snagit,
//! Screen Studio, Kap, FaceTime, and the remote-control tools TeamViewer, AnyDesk, RustDesk,
//! Quick Assist and macOS Screen Sharing (someone operating this computer sees everything).
//! Slack is deliberately NOT included (it's open all day). Match on macOS bundle id (exact) and
//! on Windows executable file name (case-insensitive exact). Display names are what the shield
//! shows ("Zoom", "Microsoft Teams", ...).
//!
//! A helper process that only exists while its app is capturing (Zoom's share host, the
//! Snipping Tool overlay) has its own entry and name. Under the app's name it would be covered
//! by a dismissal made while the app sat idle, and the start of a real share would change
//! nothing.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Platform {
    Mac,
    Windows,
}

/// One running process as reported by an OS adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningApp {
    /// macOS bundle identifier, when known.
    pub bundle_id: Option<String>,
    /// Executable file name (e.g. "Zoom.exe"), when known.
    pub exe_name: Option<String>,
}

struct KnownApp {
    name: &'static str,
    mac_bundle_ids: &'static [&'static str],
    windows_exe_names: &'static [&'static str],
}

/// Bundle ids checked against installed apps' Info.plist where available (Zoom, Discord, OBS,
/// Tencent Meeting, QuickTime, Screenshot) and public app catalogues otherwise.
const KNOWN_APPS: &[KnownApp] = &[
    KnownApp {
        name: "Zoom",
        mac_bundle_ids: &["us.zoom.xos"],
        windows_exe_names: &["Zoom.exe"],
    },
    KnownApp {
        // Zoom's screen-sharing host process: there only during a share.
        name: "Zoom screen sharing",
        mac_bundle_ids: &["us.zoom.CptHost"],
        windows_exe_names: &["CptHost.exe"],
    },
    KnownApp {
        name: "Microsoft Teams",
        // Classic Teams and new Teams ("work or school").
        mac_bundle_ids: &["com.microsoft.teams", "com.microsoft.teams2"],
        windows_exe_names: &["Teams.exe", "ms-teams.exe"],
    },
    KnownApp {
        name: "Discord",
        mac_bundle_ids: &[
            "com.hnc.Discord",
            "com.hnc.DiscordPTB",
            "com.hnc.DiscordCanary",
        ],
        windows_exe_names: &["Discord.exe", "DiscordPTB.exe", "DiscordCanary.exe"],
    },
    KnownApp {
        name: "OBS Studio",
        mac_bundle_ids: &["com.obsproject.obs-studio"],
        windows_exe_names: &["obs64.exe", "obs32.exe"],
    },
    KnownApp {
        name: "QuickTime Player",
        mac_bundle_ids: &["com.apple.QuickTimePlayerX"],
        windows_exe_names: &[],
    },
    KnownApp {
        name: "Screenshot",
        // The Screenshot app and the Cmd+Shift+5 capture UI.
        mac_bundle_ids: &["com.apple.screenshot.launcher", "com.apple.screencaptureui"],
        windows_exe_names: &[],
    },
    KnownApp {
        name: "Loom",
        mac_bundle_ids: &["com.loom.desktop"],
        windows_exe_names: &["Loom.exe"],
    },
    KnownApp {
        name: "Webex",
        // Webex App and the older Webex Meetings.
        mac_bundle_ids: &["Cisco-Systems.Spark", "com.webex.meetingmanager"],
        // WebexHost.exe hosts classic Webex Meetings (meeting controls, screen share).
        windows_exe_names: &[
            "CiscoCollabHost.exe",
            "WebexHost.exe",
            "webexmta.exe",
            "atmgr.exe",
        ],
    },
    KnownApp {
        name: "Tencent Meeting",
        // Tencent Meeting (China) and VooV Meeting (international).
        mac_bundle_ids: &[
            "com.tencent.meeting",
            "com.tencent.tencentmeeting",
            "com.tencent.meetingTencentMeeting",
        ],
        windows_exe_names: &["wemeetapp.exe", "voovmeetingapp.exe"],
    },
    KnownApp {
        name: "Skype",
        mac_bundle_ids: &["com.skype.skype"],
        windows_exe_names: &["Skype.exe"],
    },
    KnownApp {
        name: "Snipping Tool",
        mac_bundle_ids: &[],
        windows_exe_names: &["SnippingTool.exe"],
    },
    KnownApp {
        // The Win+Shift+S capture overlay: there only while capturing.
        name: "Snipping Tool capture",
        mac_bundle_ids: &[],
        windows_exe_names: &["ScreenClippingHost.exe"],
    },
    KnownApp {
        name: "Xbox Game Bar",
        mac_bundle_ids: &[],
        windows_exe_names: &["GameBar.exe"],
    },
    KnownApp {
        // The Game Bar's recording and broadcast server.
        name: "Xbox Game Bar recording",
        mac_bundle_ids: &[],
        windows_exe_names: &["bcastdvr.exe"],
    },
    KnownApp {
        name: "ShareX",
        mac_bundle_ids: &[],
        windows_exe_names: &["ShareX.exe"],
    },
    KnownApp {
        name: "CleanShot X",
        mac_bundle_ids: &[
            "pl.maketheweb.cleanshotx",
            "pl.maketheweb.cleanshotx-setapp",
        ],
        windows_exe_names: &[],
    },
    KnownApp {
        name: "Snagit",
        // The macOS bundle id carries the release year.
        mac_bundle_ids: &[
            "com.TechSmith.Snagit",
            "com.TechSmith.Snagit2018",
            "com.TechSmith.Snagit2019",
            "com.TechSmith.Snagit2020",
            "com.TechSmith.Snagit2021",
            "com.TechSmith.Snagit2022",
            "com.TechSmith.Snagit2023",
            "com.TechSmith.Snagit2024",
            "com.TechSmith.Snagit2025",
            "com.TechSmith.Snagit2026",
            "com.TechSmith.Snagit2027",
        ],
        windows_exe_names: &["Snagit32.exe", "Snagit64.exe", "SnagitCapture.exe"],
    },
    KnownApp {
        name: "Screen Studio",
        mac_bundle_ids: &["com.timpler.screenstudio"],
        windows_exe_names: &[],
    },
    KnownApp {
        name: "Kap",
        mac_bundle_ids: &["com.wulkano.kap"],
        windows_exe_names: &[],
    },
    KnownApp {
        name: "FaceTime",
        mac_bundle_ids: &["com.apple.FaceTime"],
        windows_exe_names: &[],
    },
    // Remote control: whoever is connected sees and operates this computer.
    KnownApp {
        name: "TeamViewer",
        mac_bundle_ids: &["com.teamviewer.TeamViewer"],
        windows_exe_names: &["TeamViewer.exe"],
    },
    KnownApp {
        name: "AnyDesk",
        mac_bundle_ids: &["com.philandro.anydesk"],
        windows_exe_names: &["AnyDesk.exe"],
    },
    KnownApp {
        name: "RustDesk",
        mac_bundle_ids: &["com.carriez.rustdesk"],
        windows_exe_names: &["rustdesk.exe"],
    },
    KnownApp {
        name: "Quick Assist",
        mac_bundle_ids: &[],
        windows_exe_names: &["QuickAssist.exe"],
    },
    KnownApp {
        // The agent macOS runs while someone views or controls this Mac's screen.
        name: "Screen Sharing",
        mac_bundle_ids: &["com.apple.screensharing.agent"],
        windows_exe_names: &[],
    },
];

/// Display names of known sharing/recording apps among `apps`, sorted and de-duplicated.
pub fn detect(platform: Platform, apps: &[RunningApp]) -> Vec<&'static str> {
    KNOWN_APPS
        .iter()
        .filter(|known| {
            apps.iter().any(|app| match platform {
                Platform::Mac => app
                    .bundle_id
                    .as_deref()
                    .is_some_and(|id| known.mac_bundle_ids.contains(&id)),
                Platform::Windows => app.exe_name.as_deref().is_some_and(|exe| {
                    known
                        .windows_exe_names
                        .iter()
                        .any(|k| k.eq_ignore_ascii_case(exe))
                }),
            })
        })
        .map(|known| known.name)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Shield state across polls.
#[derive(Debug, Default)]
pub struct GuardState {
    /// Apps in the latest detection.
    detected: BTreeSet<&'static str>,
    /// While a dismissal holds: the apps it covers, i.e. those running when the owner dismissed
    /// the shield that have kept running since.
    dismissed: Option<BTreeSet<&'static str>>,
}

impl GuardState {
    pub fn new() -> GuardState {
        GuardState::default()
    }

    /// Feed the latest detection. Returns whether the shield should be up.
    pub fn update(&mut self, detected: &[&'static str]) -> bool {
        self.detected = detected.iter().copied().collect();
        self.dismissed
            .take_if(|dismissed| !self.detected.is_subset(dismissed));
        // An app that quit is no longer covered: if it starts again, that is news.
        if let Some(dismissed) = &mut self.dismissed {
            dismissed.retain(|app| self.detected.contains(app));
        }
        !self.shield_apps().is_empty()
    }

    /// The owner said "I'm not sharing right now" while `detected` was running.
    pub fn dismiss(&mut self, detected: &[&'static str]) {
        self.detected = detected.iter().copied().collect();
        self.dismissed = Some(self.detected.clone());
    }

    /// Forget any dismissal (called when the Vault locks).
    pub fn reset(&mut self) {
        self.dismissed = None;
    }

    /// Apps currently causing the shield (empty if down).
    pub fn shield_apps(&self) -> BTreeSet<&'static str> {
        if self.dismissed.is_some() {
            BTreeSet::new()
        } else {
            self.detected.clone()
        }
    }
}
