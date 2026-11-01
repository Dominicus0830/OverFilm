//! When a long extraction finishes: a desktop notification, and on request putting the computer to
//! sleep (after a countdown the window shows, so someone at the desk can stop it). Both go through
//! the system's own tools (PowerShell on Windows, `notify-send`/`systemctl` on Linux, `osascript`/
//! `pmset` on macOS); nothing is linked, and a missing tool only means no notification.

use serde::{Deserialize, Serialize};

/// What to do once captions are done.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum After {
    #[default]
    Nothing,
    /// Put the computer to sleep after [`SLEEP_COUNTDOWN_SECS`].
    Sleep,
}

impl After {
    pub fn label(self) -> &'static str {
        match self {
            After::Nothing => "아무것도 안 함",
            After::Sleep => "절전 모드로",
        }
    }
}

/// Seconds the window waits (and shows a cancel button) before sleep.
pub const SLEEP_COUNTDOWN_SECS: u64 = 60;

fn short(s: &str, n: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(n).collect()
}

#[cfg(windows)]
fn powershell(script: &str, env: &[(&str, String)]) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut c = std::process::Command::new("powershell.exe");
    c.args(["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", script]).creation_flags(CREATE_NO_WINDOW);
    for (k, v) in env {
        c.env(k, v);
    }
    c.spawn().map(|_| ()).map_err(|e| format!("PowerShell을 실행하지 못했습니다: {e}"))
}

/// Show a desktop notification. The text goes through environment variables, never into the
/// script itself.
pub fn notify(title: &str, body: &str) -> Result<(), String> {
    let (title, body) = (short(title, 80), short(body, 200));
    #[cfg(windows)]
    {
        // a toast under PowerShell's own app id (always registered), else a tray balloon
        const SCRIPT: &str = r#"
$ErrorActionPreference = 'Stop'
try {
  [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] | Out-Null
  [Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime] | Out-Null
  $x = New-Object Windows.Data.Xml.Dom.XmlDocument
  $x.LoadXml('<toast><visual><binding template="ToastGeneric"><text></text><text></text></binding></visual></toast>')
  $t = $x.GetElementsByTagName('text')
  $t.Item(0).AppendChild($x.CreateTextNode($env:OF_TITLE)) | Out-Null
  $t.Item(1).AppendChild($x.CreateTextNode($env:OF_BODY)) | Out-Null
  $app = '{1AC14E77-02E7-4E5D-B744-2EB1AE5198B7}\WindowsPowerShell\v1.0\powershell.exe'
  [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier($app).Show([Windows.UI.Notifications.ToastNotification]::new($x))
} catch {
  Add-Type -AssemblyName System.Windows.Forms
  $n = New-Object System.Windows.Forms.NotifyIcon
  $n.Icon = [System.Drawing.SystemIcons]::Information
  $n.Visible = $true
  $n.ShowBalloonTip(10000, $env:OF_TITLE, $env:OF_BODY, 'Info')
  Start-Sleep -Seconds 10
  $n.Dispose()
}
"#;
        powershell(SCRIPT, &[("OF_TITLE", title), ("OF_BODY", body)])
    }
    #[cfg(target_os = "macos")]
    {
        let script = "on run argv\ndisplay notification (item 2 of argv) with title (item 1 of argv)\nend run";
        std::process::Command::new("osascript").args(["-e", script, &title, &body]).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        std::process::Command::new("notify-send").args(["--app-name=OverFilm", &title, &body]).spawn().map(|_| ()).map_err(|e| e.to_string())
    }
}

/// Put the computer to sleep now.
pub fn sleep() -> Result<(), String> {
    #[cfg(windows)]
    {
        powershell(
            "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.Application]::SetSuspendState([System.Windows.Forms.PowerState]::Suspend, $false, $false) | Out-Null",
            &[],
        )
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("pmset").arg("sleepnow").spawn().map(|_| ()).map_err(|e| e.to_string())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        std::process::Command::new("systemctl").arg("suspend").spawn().map(|_| ()).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_and_text() {
        assert_eq!(After::default(), After::Nothing);
        assert_eq!(After::Sleep.label(), "절전 모드로");
        assert_eq!(short("가\n나\u{7}다", 10), "가나다");
        assert_eq!(short(&"가".repeat(500), 200).chars().count(), 200);
        let a: After = serde_json::from_str("\"sleep\"").unwrap();
        assert_eq!(a, After::Sleep);
    }
}
