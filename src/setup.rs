//! Helping the person install asusctl: which Linux this is and the right commands for it.

use crate::ctl::Action;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Family {
    Arch,
    Fedora,
    Suse,
    Debian,
    Other,
}

#[derive(Clone, Debug)]
pub struct Distro {
    pub name: String,
    pub family: Family,
}

pub fn detect() -> Distro {
    // Inside Flatpak the host's os-release is mounted at /run/host.
    let text = std::fs::read_to_string("/run/host/os-release")
        .or_else(|_| std::fs::read_to_string("/etc/os-release"))
        .unwrap_or_default();
    let get = |k: &str| {
        text.lines()
            .find(|l| l.starts_with(&format!("{k}=")))
            .map(|l| l[k.len() + 1..].trim_matches('"').to_string())
            .unwrap_or_default()
    };
    let name = {
        let n = get("PRETTY_NAME");
        if n.is_empty() { get("NAME") } else { n }
    };
    let ids = format!("{} {}", get("ID"), get("ID_LIKE")).to_lowercase();
    let has = |w: &str| ids.split_whitespace().any(|x| x == w);
    let family = if has("arch") || has("cachyos") || has("manjaro") || has("endeavouros") {
        Family::Arch
    } else if has("fedora") || has("nobara") || has("rhel") {
        Family::Fedora
    } else if has("opensuse") || has("suse") || ids.contains("opensuse") {
        Family::Suse
    } else if has("debian") || has("ubuntu") || has("pop") || has("linuxmint") {
        Family::Debian
    } else {
        Family::Other
    };
    Distro { name: if name.is_empty() { "Unknown Linux".into() } else { name }, family }
}

const START_SERVICE: &str = "(systemctl enable --now asusd 2>/dev/null || systemctl start asusd)";

/// Cardwire (instant Eco ↔ Standard switching): installed from the distribution's own
/// repositories when one carries it (CachyOS does), and its service started.
const CARDWIRE_FROM_REPO: &str = "if pacman -Si cardwire >/dev/null 2>&1; then \
     pacman -S --needed --noconfirm cardwire && systemctl enable --now cardwired; fi";

pub const CARDWIRE_URL: &str = "https://github.com/OpenGamingCollective/cardwire";

/// Commands run as root by "Install for me" (through pkexec). None when there is no package.
/// On Arch-family systems Cardwire comes along when a repository has it.
pub fn install_script(f: Family) -> Option<String> {
    match f {
        Family::Arch => Some(format!(
            "pacman -S --needed --noconfirm asusctl && {START_SERVICE}; {CARDWIRE_FROM_REPO}"
        )),
        Family::Fedora => Some(format!(
            "dnf copr enable -y lukenukem/asus-linux && dnf install -y asusctl && {START_SERVICE}"
        )),
        Family::Suse => Some(format!(
            "zypper --non-interactive ar -f -p 50 https://download.opensuse.org/repositories/home:/luke_nukem:/asus/openSUSE_Tumbleweed/ asus-linux; \
             zypper --non-interactive --gpg-auto-import-keys ref && zypper --non-interactive in asusctl && {START_SERVICE}"
        )),
        Family::Debian | Family::Other => None,
    }
}

/// The same thing for a terminal, with sudo, for "Copy command" — Cardwire included where it
/// can be installed (`cardwire`: how, from `cardwire_install`).
pub fn manual_command(f: Family, cardwire: Option<&CardwireInstall>) -> String {
    match (f, cardwire) {
        (Family::Arch, Some(CardwireInstall::Repo)) => {
            "sudo pacman -S --needed asusctl cardwire && sudo systemctl enable --now asusd cardwired".into()
        }
        (Family::Arch, Some(CardwireInstall::Aur(helper))) => format!(
            "sudo pacman -S --needed asusctl && sudo systemctl enable --now asusd && \
             {helper} -S --needed cardwire && sudo systemctl enable --now cardwired"
        ),
        _ => manual_command_asusctl(f),
    }
}

fn manual_command_asusctl(f: Family) -> String {
    match f {
        Family::Arch => "sudo pacman -S --needed asusctl && sudo systemctl enable --now asusd".into(),
        Family::Fedora => "sudo dnf copr enable lukenukem/asus-linux && sudo dnf install asusctl && \
             sudo systemctl enable --now asusd"
            .into(),
        Family::Suse => "sudo zypper ar -f https://download.opensuse.org/repositories/home:/luke_nukem:/asus/openSUSE_Tumbleweed/ asus-linux && \
             sudo zypper ref && sudo zypper in asusctl && sudo systemctl enable --now asusd"
            .into(),
        Family::Debian | Family::Other => {
            "No official package for this Linux. Follow the build guide at https://asus-linux.org".into()
        }
    }
}

pub fn install_action(f: Family) -> Option<Action> {
    let script = install_script(f)?;
    Some(Action {
        label: "Install asusctl".into(),
        groups: vec![vec![vec!["pkexec".into(), "sh".into(), "-c".into(), script]]],
        ..Default::default()
    })
}

/// How Cardwire can be installed on this system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CardwireInstall {
    /// A configured repository carries it: pacman behind one password prompt.
    Repo,
    /// Only the AUR has it: a terminal opens with this helper (it asks for the password there).
    Aur(String),
    /// No package for this Linux: point to the project page.
    Manual,
}

pub fn cardwire_install(f: Family) -> CardwireInstall {
    if f != Family::Arch {
        return CardwireInstall::Manual;
    }
    if crate::ctl::run_host(&["pacman", "-Si", "cardwire"]).is_ok() {
        return CardwireInstall::Repo;
    }
    for helper in ["paru", "yay"] {
        if crate::ctl::run_host(&["sh", "-c", &format!("command -v {helper}")]).is_ok() {
            return CardwireInstall::Aur(helper.into());
        }
    }
    CardwireInstall::Manual
}

pub fn cardwire_action(how: &CardwireInstall) -> Option<Action> {
    let enable = "systemctl --no-ask-password enable --now cardwired 2>/dev/null || sudo systemctl enable --now cardwired";
    let cmd: Vec<String> = match how {
        CardwireInstall::Repo => vec![
            "pkexec".into(),
            "sh".into(),
            "-c".into(),
            "pacman -S --needed --noconfirm cardwire && systemctl enable --now cardwired".into(),
        ],
        CardwireInstall::Aur(helper) => {
            // AUR packages are built as the user and installed with sudo, which needs a
            // terminal for the password.
            let script = format!(
                "{helper} -S --needed cardwire && ({enable}); echo; read -p 'Done - press Enter to close.' _"
            );
            terminal_command(&script)?
        }
        CardwireInstall::Manual => return None,
    };
    Some(Action { label: "Install Cardwire".into(), groups: vec![vec![cmd]], ..Default::default() })
}

/// A terminal window running `script`: the first terminal program found.
fn terminal_command(script: &str) -> Option<Vec<String>> {
    let found = |t: &str| crate::ctl::run_host(&["sh", "-c", &format!("command -v {t}")]).is_ok();
    let s = script.to_string();
    let cmd: Vec<&str> = if found("konsole") {
        vec!["konsole", "-e", "sh", "-c"]
    } else if found("kgx") {
        vec!["kgx", "--", "sh", "-c"]
    } else if found("gnome-terminal") {
        vec!["gnome-terminal", "--wait", "--", "sh", "-c"]
    } else if found("alacritty") {
        vec!["alacritty", "-e", "sh", "-c"]
    } else if found("kitty") {
        vec!["kitty", "sh", "-c"]
    } else if found("xterm") {
        vec!["xterm", "-e", "sh", "-c"]
    } else {
        return None;
    };
    let mut v: Vec<String> = cmd.into_iter().map(String::from).collect();
    v.push(s);
    Some(v)
}

pub fn start_service_action() -> Action {
    Action {
        label: "Start the asusctl service".into(),
        groups: vec![vec![vec!["pkexec".into(), "sh".into(), "-c".into(), START_SERVICE.into()]]],
        ..Default::default()
    }
}
