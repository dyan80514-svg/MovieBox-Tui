pub mod apply;
pub mod artifact;
pub mod check;
pub mod download;
pub mod extract;
pub mod verify;

pub use apply::{
    InstallationEnvironment, SelfUpdateOutcome, apply_staged_binary, detect_environment,
    is_homebrew_managed, is_scoop_managed, is_writable, resolve_executable_path, restart_process,
};
pub use artifact::{Release, ReleaseAsset, TargetPlatform, is_termux_environment};
pub use check::{check, check_release, is_newer};

pub async fn perform_self_update(
    release: &Release,
    progress_sender: Option<&tokio::sync::mpsc::UnboundedSender<String>>,
) -> Result<SelfUpdateOutcome, String> {
    let current_exe = std::env::current_exe()
        .map_err(|e| format!("failed to get current executable path: {e}"))?;

    let env = detect_environment(&current_exe);
    if let Some(instruction) = env.upgrade_instruction() {
        return Ok(SelfUpdateOutcome::RequiresManualUpgrade(
            instruction.to_string(),
        ));
    }

    let platform = TargetPlatform::current().ok_or_else(|| {
        "current operating system or architecture is not supported for in-app self-update"
            .to_string()
    })?;

    let asset = release.find_compatible_asset(platform).ok_or_else(|| {
        format!(
            "no compatible release asset found for platform {:?}",
            platform
        )
    })?;

    let checksum_asset = release
        .find_checksum_asset()
        .ok_or_else(|| "no SHA256SUMS checksum file found in GitHub release".to_string())?;
    let temp_dir = tempfile::Builder::new()
        .prefix("moviebox_update_")
        .tempdir()
        .map_err(|e| format!("failed to create temporary update directory: {e}"))?;

    let archive_path = temp_dir.path().join(&asset.name);
    let staged_binary_path = apply::persistent_staging_path(&current_exe);

    if let Some(tx) = progress_sender {
        let _ = tx.send(format!("Downloading {}...", asset.name));
    }
    download::download_file(&asset.download_url, &archive_path).await?;

    if let Some(tx) = progress_sender {
        let _ = tx.send("Downloading SHA256SUMS...".to_string());
    }
    let sha256sums_content = download::download_text(&checksum_asset.download_url).await?;

    if let Some(tx) = progress_sender {
        let _ = tx.send("Verifying SHA-256 integrity...".to_string());
    }
    verify::verify_checksum(&archive_path, &sha256sums_content, &asset.name)?;

    if let Some(tx) = progress_sender {
        let _ = tx.send("Extracting binary from release archive...".to_string());
    }
    extract::extract_binary(
        &archive_path,
        &asset.name,
        platform.expected_binary_name(),
        &staged_binary_path,
    )?;

    if let Some(tx) = progress_sender {
        let _ = tx.send("Applying update to executable...".to_string());
    }
    let outcome = apply_staged_binary(&staged_binary_path, &current_exe)?;

    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_newer_comprehensive_matrix() {
        for (current, newer) in [
            ("0.1.9", "0.1.10"),
            ("0.1.10", "0.1.11"),
            ("0.1.12", "0.1.13"),
            ("1.9.0", "1.10.0"),
            ("0.9.9", "1.0.0"),
            ("v0.1.9", "v0.1.10"),
            ("0.1.9", "v0.1.10"),
            ("v0.1.9", "0.1.10"),
            ("1.0.0-beta", "1.0.0"),
            ("1.0.0-rc.1", "1.0.0"),
            ("1.0.0-beta.1", "1.0.0-beta.2"),
        ] {
            assert!(is_newer(current, newer), "{current} -> {newer}");
        }
        for (current, other) in [
            ("0.1.12", "0.1.12"),
            ("1.0.0", "1.0.0"),
            ("0.1.13", "0.1.12"),
            ("1.10.0", "1.9.0"),
            ("2.0.0", "1.9.9"),
            ("v0.1.12", "v0.1.12"),
            ("v0.1.13", "v0.1.12"),
            ("1.0.0", "1.0.0-beta"),
        ] {
            assert!(!is_newer(current, other), "{current} -> {other}");
        }
    }

    #[test]
    fn test_is_newer_non_semver_fails_closed() {
        assert!(!is_newer("0.1.12", "custom-build-1"));
        assert!(!is_newer("custom-build-1", "custom-build-1"));
        assert!(!is_newer("custom-build-1", "0.1.27"));
    }

    #[test]
    fn test_release_json_deserialization() {
        let raw_json = "{\"tag_name\": \"v0.1.13\", \"body\": \"### New Features\\n- Added cool feature\\n- Fixed bugs\"}";
        let item: serde_json::Value = serde_json::from_str(raw_json).unwrap();
        let tag = item["tag_name"].as_str().unwrap();
        let release = Release {
            version: tag.trim_start_matches('v').to_string(),
            tag_name: tag.to_string(),
            notes: item["body"].as_str().unwrap_or("").to_string(),
            assets: Vec::new(),
        };
        assert_eq!(release.version, "0.1.13");
        assert!(release.notes.contains("### New Features"));
    }

    #[test]
    fn test_release_json_missing_body_graceful() {
        let item: serde_json::Value =
            serde_json::from_str("{\"tag_name\": \"v0.1.13\"}").unwrap();
        assert_eq!(item["body"].as_str().unwrap_or(""), "");
        assert_eq!(item["tag_name"].as_str().unwrap(), "v0.1.13");
    }

    fn sample_release_with_all_assets() -> Release {
        let names = [
            "MovieBox_macOS_Universal.tar.gz",
            "MovieBox_Linux_x64.tar.gz",
            "MovieBox_Linux_arm64.tar.gz",
            "MovieBox_Windows_x64.zip",
            "MovieBox_Windows_arm64.zip",
            "SHA256SUMS",
        ];
        Release {
            version: "0.1.13".to_string(),
            tag_name: "v0.1.13".to_string(),
            notes: "Bug fixes and improvements".to_string(),
            assets: names
                .into_iter()
                .map(|name| ReleaseAsset {
                    name: name.to_string(),
                    download_url: format!(
                        "https://github.com/{}/{}/releases/download/v0.1.13/{name}",
                        check::OWNER,
                        check::REPOSITORY
                    ),
                    size: Some(1000),
                })
                .collect(),
        }
    }

    #[test]
    fn test_update_asset_selection_for_current_platform() {
        let release = sample_release_with_all_assets();
        for (os, arch, expected) in [
            ("macos", "aarch64", "MovieBox_macOS_Universal.tar.gz"),
            ("linux", "x86_64", "MovieBox_Linux_x64.tar.gz"),
            ("linux", "aarch64", "MovieBox_Linux_arm64.tar.gz"),
            ("windows", "x86_64", "MovieBox_Windows_x64.zip"),
            ("windows", "arm64", "MovieBox_Windows_arm64.zip"),
        ] {
            let platform = TargetPlatform::detect(os, arch, false).unwrap();
            assert_eq!(release.find_compatible_asset(platform).unwrap().name, expected);
        }
        assert_eq!(release.find_checksum_asset().unwrap().name, "SHA256SUMS");
    }

    #[test]
    fn test_update_asset_missing_for_current_platform() {
        let mut release = sample_release_with_all_assets();
        release.assets.retain(|asset| asset.name == "MovieBox_Linux_x64.tar.gz");
        assert!(release.find_compatible_asset(TargetPlatform::MacosUniversal).is_none());
        assert!(release.find_compatible_asset(TargetPlatform::WindowsX64).is_none());
        assert!(release.find_checksum_asset().is_none());
    }

    #[test]
    fn test_update_asset_rejects_wrong_architecture() {
        assert!(TargetPlatform::detect("linux", "mips", false).is_none());
        assert!(TargetPlatform::detect("linux", "riscv64", false).is_none());
        assert!(TargetPlatform::detect("windows", "ia64", false).is_none());
    }

    #[test]
    fn test_update_asset_rejects_wrong_platform() {
        assert!(TargetPlatform::detect("freebsd", "x86_64", false).is_none());
        assert!(TargetPlatform::detect("openbsd", "x86_64", false).is_none());
        assert!(TargetPlatform::detect("solaris", "x86_64", false).is_none());
    }

    #[test]
    fn test_update_asset_termux_is_protected_from_glibc_binary() {
        assert!(TargetPlatform::detect("linux", "aarch64", true).is_none());
        assert!(TargetPlatform::detect("linux", "x86_64", true).is_none());
        assert!(TargetPlatform::detect("android", "aarch64", false).is_none());
    }

    #[test]
    fn test_update_asset_multiple_candidates_deterministic() {
        let release = sample_release_with_all_assets();
        let platform = TargetPlatform::LinuxX64;
        let first = release.find_compatible_asset(platform);
        assert_eq!(first, release.find_compatible_asset(platform));
        assert_eq!(first.unwrap().name, "MovieBox_Linux_x64.tar.gz");
    }
}
