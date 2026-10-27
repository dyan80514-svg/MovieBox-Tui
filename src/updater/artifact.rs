use super::check::{OWNER, REPOSITORY};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub download_url: String,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Release {
    pub version: String,
    pub tag_name: String,
    pub notes: String,
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetPlatform {
    MacosUniversal,
    LinuxX64,
    LinuxArm64,
    WindowsX64,
    WindowsArm64,
}

impl TargetPlatform {
    pub fn current() -> Option<Self> {
        Self::detect(
            std::env::consts::OS,
            std::env::consts::ARCH,
            is_termux_environment(),
        )
    }

    pub fn detect(os: &str, arch: &str, is_termux: bool) -> Option<Self> {
        if is_termux || os == "android" {
            return None;
        }
        match os {
            "macos" | "darwin" => match arch {
                "x86_64" | "x86-64" | "x64" | "amd64" | "aarch64" | "arm64" => {
                    Some(Self::MacosUniversal)
                }
                _ => None,
            },
            "linux" => match arch {
                "x86_64" | "x86-64" | "x64" | "amd64" => Some(Self::LinuxX64),
                "aarch64" | "arm64" => Some(Self::LinuxArm64),
                _ => None,
            },
            "windows" => match arch {
                "x86_64" | "x86-64" | "x64" | "amd64" => Some(Self::WindowsX64),
                "aarch64" | "arm64" => Some(Self::WindowsArm64),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn expected_asset_name(self) -> &'static str {
        match self {
            Self::MacosUniversal => "MovieBox_macOS_Universal.tar.gz",
            Self::LinuxX64 => "MovieBox_Linux_x64.tar.gz",
            Self::LinuxArm64 => "MovieBox_Linux_arm64.tar.gz",
            Self::WindowsX64 => "MovieBox_Windows_x64.zip",
            Self::WindowsArm64 => "MovieBox_Windows_arm64.zip",
        }
    }

    pub fn expected_binary_name(self) -> &'static str {
        match self {
            Self::WindowsX64 | Self::WindowsArm64 => "moviebox-tui.exe",
            _ => "moviebox-tui",
        }
    }
}

pub use crate::config::{TERMUX_PREFIX_USR, is_termux_environment};

impl Release {
    fn find_asset(&self, expected: &str) -> Option<ReleaseAsset> {
        if let Some(asset) = self.assets.iter().find(|asset| asset.name == expected) {
            return Some(asset.clone());
        }
        if !self.assets.is_empty() {
            return None;
        }
        let version = self.tag_name.strip_prefix('v').unwrap_or(&self.tag_name);
        semver::Version::parse(version).ok()?;
        Some(ReleaseAsset {
            name: expected.to_string(),
            download_url: format!(
                "https://github.com/{OWNER}/{REPOSITORY}/releases/download/{}/{expected}",
                self.tag_name
            ),
            size: None,
        })
    }

    pub fn find_compatible_asset(&self, platform: TargetPlatform) -> Option<ReleaseAsset> {
        self.find_asset(platform.expected_asset_name())
    }

    pub fn find_checksum_asset(&self) -> Option<ReleaseAsset> {
        self.find_asset("SHA256SUMS")
    }

    pub fn is_compatible_with_current_platform(&self) -> bool {
        TargetPlatform::current()
            .and_then(|platform| self.find_compatible_asset(platform))
            .is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fallback_release(tag: &str) -> Release {
        Release {
            version: "0.1.16".to_string(),
            tag_name: tag.to_string(),
            notes: String::new(),
            assets: Vec::new(),
        }
    }

    #[test]
    fn test_synthetic_asset_url_generation_on_empty_assets() {
        let release = fallback_release("v0.1.16");
        for platform in [TargetPlatform::LinuxX64, TargetPlatform::MacosUniversal] {
            let asset = release.find_compatible_asset(platform).unwrap();
            assert_eq!(asset.name, platform.expected_asset_name());
            assert_eq!(
                asset.download_url,
                format!(
                    "https://github.com/dyan80514-svg/MovieBox-Tui/releases/download/v0.1.16/{}",
                    platform.expected_asset_name()
                )
            );
        }
        let checksum = release.find_checksum_asset().unwrap();
        assert_eq!(checksum.name, "SHA256SUMS");
        assert_eq!(
            checksum.download_url,
            "https://github.com/dyan80514-svg/MovieBox-Tui/releases/download/v0.1.16/SHA256SUMS"
        );
    }

    #[test]
    fn invalid_tags_do_not_generate_synthetic_assets() {
        for tag in ["", "latest", "../../other", "v0.1.16?redirect=elsewhere"] {
            let release = fallback_release(tag);
            assert!(release.find_compatible_asset(TargetPlatform::LinuxX64).is_none());
            assert!(release.find_checksum_asset().is_none());
        }
    }

    #[test]
    fn unsupported_macos_architecture_is_rejected() {
        assert!(TargetPlatform::detect("macos", "powerpc", false).is_none());
    }
}
