use std::time::Duration;

use anyhow::{Context, Result};
use blockitall_update::{Config, InstallConfig, Updater, manifest::TrustedKey};

#[used]
pub static COMMIT_TAG: &str = concat!("AITIERLIST_COMMIT=", env!("AITIERLIST_COMMIT"), "\0");

#[used]
pub static TREE_TAG: &str = concat!("AITIERLIST_TREE=", env!("AITIERLIST_TREE"), "\0");

pub fn install_config() -> Result<InstallConfig> {
    let directories = directories::ProjectDirs::from("us", "BlockItAll", "aitierlist")
        .context("the application data directory is unavailable")?;
    let mut config = InstallConfig::new(
        "aitierlist",
        directories.data_local_dir(),
        env!("CARGO_PKG_VERSION"),
    );
    config.required_stamp = Some("AITIERLIST_VERSION=".to_owned());
    Ok(config)
}

pub fn updater(install: InstallConfig) -> Result<Updater> {
    let trusted = TrustedKey::from_json(include_str!("../public-key.json"))?;
    Updater::new(Config {
        install,
        manifest_url: "https://files.blockitall.us/aitierlist/update.json".to_owned(),
        allowed_origin: "https://files.blockitall.us".to_owned(),
        artifact_kind: "portable".to_owned(),
        architectures: vec!["x64".to_owned()],
        user_agent: format!("aitierlist/{}", env!("CARGO_PKG_VERSION")),
        trusted_keys: vec![trusted],
        check_interval: Duration::from_secs(24 * 60 * 60),
    })
}
