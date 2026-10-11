//! Putting the computer to sleep or turning it off when downloads are done.

use crate::queue::PowerAction;

/// The program and arguments that carry out `action` on this system.
pub fn command(action: PowerAction) -> Option<(&'static str, Vec<&'static str>)> {
    #[cfg(windows)]
    let command = match action {
        PowerAction::Shutdown => Some(("shutdown", vec!["/s", "/t", "0"])),
        PowerAction::Hibernate => Some(("shutdown", vec!["/h"])),
        // Sleep, not hibernate, whatever the hibernation setting says.
        PowerAction::Sleep => Some((
            "powershell",
            vec![
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Add-Type -AssemblyName System.Windows.Forms; \
                 [System.Windows.Forms.Application]::SetSuspendState('Suspend', $false, $false)",
            ],
        )),
        PowerAction::Nothing | PowerAction::Quit => None,
    };
    #[cfg(target_os = "macos")]
    let command = match action {
        PowerAction::Shutdown => Some((
            "osascript",
            vec!["-e", "tell application \"System Events\" to shut down"],
        )),
        PowerAction::Sleep | PowerAction::Hibernate => Some(("pmset", vec!["sleepnow"])),
        PowerAction::Nothing | PowerAction::Quit => None,
    };
    #[cfg(not(any(windows, target_os = "macos")))]
    let command = match action {
        PowerAction::Shutdown => Some(("systemctl", vec!["poweroff"])),
        PowerAction::Sleep => Some(("systemctl", vec!["suspend"])),
        PowerAction::Hibernate => Some(("systemctl", vec!["hibernate"])),
        PowerAction::Nothing | PowerAction::Quit => None,
    };
    command
}

/// Carries out `action`. Quitting SDM is up to the caller.
pub async fn perform(action: PowerAction) -> Result<(), String> {
    let Some((program, args)) = command(action) else {
        return Ok(());
    };
    let mut command = tokio::process::Command::new(program);
    command.args(&args);
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let status = command
        .status()
        .await
        .map_err(|error| format!("could not run {program}: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} failed ({status})"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_power_action_has_a_command() {
        assert!(command(PowerAction::Nothing).is_none());
        assert!(command(PowerAction::Quit).is_none());
        for action in [
            PowerAction::Sleep,
            PowerAction::Hibernate,
            PowerAction::Shutdown,
        ] {
            assert!(command(action).is_some(), "{action:?}");
        }
    }
}
