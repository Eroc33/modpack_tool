pub mod upgrade;
mod add;
pub use add::add;

use crate::{
    mod_source::ModpackConfig,
};
use failure::ResultExt;

pub async fn upgrade(mc_version: Option<String>, pack_path: String, pack: ModpackConfig) -> Result<(), crate::Error>{
    if let Some(ver) = mc_version{
        let ver = if ver.chars()
        .next()
        .expect("mc_version should not have length 0 due to arg parser")
        .is_numeric()
        {
            //interpret a versionreq of x as ~x
            println!("Interpreting version {} as ~{}", ver, ver);
            format!("~{}", ver)
        } else {
            ver.to_owned()
        };
        let ver = semver::VersionReq::parse(ver.as_str()).context(format!(
            "Second argument ({}) was not a semver version requirement",
            ver
        ))?;
        upgrade::new_version(
            ver,
            pack_path.to_owned(),
            pack,
        ).await
    }else{
        let release_status = pack.auto_update_release_status
            .ok_or(crate::Error::AutoUpdateDisabled)
            .context(format!(
                "Pack {} has no auto_update_release_status",
                pack_path
            ))?;
        upgrade::same_version(
            pack_path.to_owned(),
            pack,
            release_status,
        ).await
    }
}