//! Helpers to display version information

use clap::crate_version;

pub(crate) struct Version;

impl Version {
    pub(crate) fn long() -> &'static str {
        let crate_version = crate_version!();
        let git_hash = env!("GIT_HASH");
        let message = format!("{crate_version}\n\nCompiled from (git hash): {git_hash}");
        Box::leak(message.into_boxed_str())
    }

    pub(crate) fn short() -> &'static str {
        let crate_version = crate_version!();
        let git_hash = env!("GIT_HASH");
        let message = format!("Version {crate_version}, hash: {git_hash}");
        Box::leak(message.into_boxed_str())
    }

    pub(crate) fn crate_version() -> &'static str {
        crate_version!()
    }

    pub(crate) fn git_hash() -> &'static str {
        env!("GIT_HASH")
    }
}
