//! Preinstalled Microsoft Store (AppX) bloat removal.
//!
//! Three tiers: curated bloat (pre-selected), optional apps some people use (not
//! pre-selected) and everything else. A protected list can never be removed.

use crate::util::wildcard_match;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Category {
    Bloat,
    Optional,
    Other,
}

impl Category {
    pub fn label(self) -> &'static str {
        match self {
            Category::Bloat => "Recommended to remove",
            Category::Optional => "Optional",
            Category::Other => "Other apps",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AppxPackage {
    pub name: String,
    pub full_name: String,
    pub version: String,
    pub category: Category,
    pub protected: bool,
    pub friendly: String,
}

/// Preinstalled apps almost nobody needs.
const BLOAT: &[(&str, &str)] = &[
    ("Microsoft.BingNews", "Microsoft News"),
    ("Microsoft.BingWeather", "Weather"),
    ("Microsoft.BingFinance", "Money"),
    ("Microsoft.BingSports", "Sports"),
    ("Microsoft.BingTranslator", "Translator"),
    ("Microsoft.BingFoodAndDrink", "Food & Drink"),
    ("Microsoft.BingHealthAndFitness", "Health & Fitness"),
    ("Microsoft.BingTravel", "Travel"),
    ("Microsoft.News", "Microsoft News"),
    ("Microsoft.GetHelp", "Get Help"),
    ("Microsoft.Getstarted", "Tips"),
    ("Microsoft.MicrosoftSolitaireCollection", "Solitaire Collection"),
    ("Microsoft.MicrosoftOfficeHub", "Microsoft 365 (Office) hub"),
    ("Microsoft.People", "People"),
    ("Microsoft.WindowsFeedbackHub", "Feedback Hub"),
    ("Microsoft.ZuneVideo", "Movies & TV"),
    ("Microsoft.MixedReality.Portal", "Mixed Reality Portal"),
    ("Microsoft.SkypeApp", "Skype"),
    ("Microsoft.Microsoft3DViewer", "3D Viewer"),
    ("Microsoft.MSPaint", "Paint 3D"),
    ("Microsoft.Print3D", "Print 3D"),
    ("Microsoft.Messaging", "Messaging"),
    ("Microsoft.OneConnect", "Mobile Plans"),
    ("Microsoft.Wallet", "Wallet"),
    ("Microsoft.549981C3F5F10", "Cortana"),
    ("Microsoft.WindowsMaps", "Maps"),
    ("Microsoft.Office.Sway", "Sway"),
    ("Microsoft.NetworkSpeedTest", "Network Speed Test"),
    ("Microsoft.Windows.DevHome", "Dev Home"),
    ("Clipchamp.Clipchamp", "Clipchamp"),
    ("MicrosoftCorporationII.MicrosoftFamily", "Microsoft Family"),
    ("king.com.*", "King games (Candy Crush…)"),
    ("*CandyCrush*", "Candy Crush"),
    ("*BubbleWitch*", "Bubble Witch"),
    ("*MarchofEmpires*", "March of Empires"),
    ("*HiddenCity*", "Hidden City"),
    ("*FarmVille*", "FarmVille"),
    ("*Asphalt*", "Asphalt"),
    ("*RoyalRevolt*", "Royal Revolt"),
    ("Facebook.*", "Facebook"),
    ("*.Facebook", "Facebook"),
    ("*Twitter*", "Twitter / X"),
    ("BytedancePte.Ltd.TikTok", "TikTok"),
    ("*TikTok*", "TikTok"),
    ("*Instagram*", "Instagram"),
    ("AmazonVideo.PrimeVideo", "Prime Video"),
    ("Disney.*", "Disney+"),
    ("*Netflix*", "Netflix"),
    ("*Hulu*", "Hulu"),
    ("*Pandora*", "Pandora"),
    ("*Shazam*", "Shazam"),
    ("*Flipboard*", "Flipboard"),
    ("*PicsArt*", "PicsArt"),
    ("*LinkedIn*", "LinkedIn"),
    ("*SlingTV*", "Sling TV"),
    ("*Duolingo*", "Duolingo"),
];

/// Apps some people use; shown but not pre-selected.
const OPTIONAL: &[(&str, &str)] = &[
    ("Microsoft.XboxApp", "Xbox Console Companion"),
    ("Microsoft.GamingApp", "Xbox"),
    ("Microsoft.XboxGamingOverlay", "Xbox Game Bar"),
    ("Microsoft.XboxGameOverlay", "Xbox Game Overlay"),
    ("Microsoft.XboxSpeechToTextOverlay", "Xbox speech overlay"),
    ("Microsoft.XboxIdentityProvider", "Xbox Identity Provider"),
    ("Microsoft.Xbox.TCUI", "Xbox TCUI"),
    ("Microsoft.YourPhone", "Phone Link"),
    ("MicrosoftTeams", "Teams (personal)"),
    ("MSTeams", "Microsoft Teams"),
    ("Microsoft.OutlookForWindows", "Outlook (new)"),
    ("microsoft.windowscommunicationsapps", "Mail and Calendar"),
    ("Microsoft.Todos", "Microsoft To Do"),
    ("Microsoft.MicrosoftStickyNotes", "Sticky Notes"),
    ("Microsoft.PowerAutomateDesktop", "Power Automate"),
    ("Microsoft.WindowsAlarms", "Clock"),
    ("Microsoft.WindowsSoundRecorder", "Sound Recorder"),
    ("Microsoft.WindowsCamera", "Camera"),
    ("Microsoft.ZuneMusic", "Media Player"),
    ("Microsoft.BingSearch", "Bing Search"),
    ("Microsoft.Copilot", "Copilot"),
    ("Microsoft.MicrosoftJournal", "Journal"),
    ("MicrosoftCorporationII.QuickAssist", "Quick Assist"),
    ("Microsoft.Office.OneNote", "OneNote for Windows 10"),
    ("*Spotify*", "Spotify"),
    ("*Dolby*", "Dolby Access"),
    ("*WhatsApp*", "WhatsApp"),
    ("*Minecraft*", "Minecraft"),
    ("*Roblox*", "Roblox"),
];

/// Never removable: core Windows experience, Store, security, codecs, runtimes.
const PROTECTED: &[&str] = &[
    "Microsoft.WindowsStore", "Microsoft.StorePurchaseApp", "Microsoft.DesktopAppInstaller",
    "Microsoft.SecHealthUI", "Microsoft.Windows.SecHealthUI", "Microsoft.Windows.Photos",
    "Microsoft.WindowsNotepad", "Microsoft.WindowsTerminal", "Microsoft.Paint", "Microsoft.ScreenSketch",
    "Microsoft.WindowsCalculator", "Microsoft.Winget.Source", "Microsoft.WindowsAppRuntime*",
    "Microsoft.UI.Xaml*", "Microsoft.VCLibs*", "Microsoft.NET.*", "Microsoft.Services.Store.Engagement",
    "Microsoft.*Extension*", "Microsoft.*VideoExtension*", "Microsoft.WebMediaExtensions",
    "Microsoft.Windows.ShellExperienceHost", "Microsoft.Windows.StartMenuExperienceHost",
    "MicrosoftWindows.Client.*", "Microsoft.AAD.BrokerPlugin", "Microsoft.AccountsControl",
    "Microsoft.LockApp", "Microsoft.Windows.CloudExperienceHost", "Microsoft.Win32WebViewHost",
    "Microsoft.MicrosoftEdge*", "windows.immersivecontrolpanel", "Microsoft.Windows.*",
    "Microsoft.WindowsAppSDK*", "MicrosoftWindows.*", "Microsoft.ApplicationCompatibilityEnhancements",
    "Microsoft.CredDialogHost", "Microsoft.ECApp", "Microsoft.BioEnrollment", "NcsiUwpApp",
    "Microsoft.OneDriveSync", "Microsoft.Wallet*Extension", "Microsoft.DirectXRuntime",
];

fn find(list: &[(&str, &str)], name: &str) -> Option<String> {
    list.iter().find(|(p, _)| wildcard_match(p, name)).map(|(_, f)| f.to_string())
}

pub fn classify(name: &str, system_signed: bool) -> (Category, bool, String) {
    if let Some(f) = find(BLOAT, name) {
        return (Category::Bloat, false, f);
    }
    let protected = PROTECTED.iter().any(|p| wildcard_match(p, name));
    if let Some(f) = find(OPTIONAL, name) {
        return (Category::Optional, protected, f);
    }
    (Category::Other, protected || system_signed, name.to_string())
}

pub fn list() -> Result<Vec<AppxPackage>, String> {
    if !cfg!(windows) {
        return Ok(Vec::new());
    }
    let out = crate::sys::powershell(
        "$ErrorActionPreference='Stop'; Get-AppxPackage | Where-Object { -not $_.IsFramework -and -not $_.NonRemovable } | \
         Select-Object Name, PackageFullName, @{n='Version';e={[string]$_.Version}}, @{n='Sig';e={[string]$_.SignatureKind}} | ConvertTo-Json -Compress",
    )?;
    let mut pkgs: Vec<AppxPackage> = crate::sys::json_list(&out)
        .iter()
        .filter_map(|v| {
            let name = v["Name"].as_str()?.to_string();
            let (category, protected, friendly) = classify(&name, v["Sig"].as_str() == Some("System"));
            Some(AppxPackage {
                full_name: v["PackageFullName"].as_str()?.to_string(),
                version: v["Version"].as_str().unwrap_or("").to_string(),
                name,
                category,
                protected,
                friendly,
            })
        })
        .collect();
    pkgs.sort_by(|a, b| a.category.cmp(&b.category).then(a.friendly.to_lowercase().cmp(&b.friendly.to_lowercase())));
    Ok(pkgs)
}

/// Remove packages for the current user, optionally also for new users (deprovision).
/// Returns per-package errors (empty on full success).
pub fn remove(pkgs: &[AppxPackage], deprovision: bool) -> Vec<(String, String)> {
    let mut errors = Vec::new();
    for p in pkgs {
        if p.protected {
            errors.push((p.friendly.clone(), "protected Windows component".into()));
            continue;
        }
        let mut script = format!(
            "$ErrorActionPreference='Stop'; Remove-AppxPackage -Package {}",
            crate::sys::ps_quote(&p.full_name)
        );
        if deprovision {
            script.push_str(&format!(
                "; Get-AppxProvisionedPackage -Online | Where-Object DisplayName -eq {} | \
                 Remove-AppxProvisionedPackage -Online -ErrorAction SilentlyContinue | Out-Null",
                crate::sys::ps_quote(&p.name)
            ));
        }
        if let Err(e) = crate::sys::powershell(&script) {
            errors.push((p.friendly.clone(), e));
        }
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classification() {
        assert_eq!(classify("Microsoft.BingWeather", false).0, Category::Bloat);
        assert_eq!(classify("king.com.CandyCrushSaga", false).0, Category::Bloat);
        assert_eq!(classify("Microsoft.GamingApp", false).0, Category::Optional);
        let (cat, protected, _) = classify("Microsoft.WindowsStore", false);
        assert_eq!(cat, Category::Other);
        assert!(protected);
        assert!(classify("Microsoft.HEVCVideoExtension", false).1);
        assert!(classify("Microsoft.Windows.Photos", false).1);
        assert!(classify("SomeVendor.SystemThing", true).1);
        assert!(!classify("SomeVendor.Game", false).1);
    }
}
