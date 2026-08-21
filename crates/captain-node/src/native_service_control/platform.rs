use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

#[derive(Clone)]
pub(super) enum NativeServicePlatform {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Launchd {
        domain: String,
        target: String,
        definition: PathBuf,
    },
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    SystemdUser { definition: PathBuf },
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    Windows,
}

impl NativeServicePlatform {
    pub(super) fn detect(user_home: &Path) -> Result<Self, super::NodeNativeServiceError> {
        #[cfg(target_os = "macos")]
        {
            let uid = unsafe { libc::geteuid() };
            let domain = format!("gui/{uid}");
            return Ok(Self::Launchd {
                target: format!("{domain}/{}", crate::NODE_LAUNCHD_LABEL),
                domain,
                definition: user_home
                    .join("Library")
                    .join("LaunchAgents")
                    .join(format!("{}.plist", crate::NODE_LAUNCHD_LABEL)),
            });
        }
        #[cfg(target_os = "linux")]
        {
            let config_home = std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .filter(|path| path.is_absolute())
                .unwrap_or_else(|| user_home.join(".config"));
            return Ok(Self::SystemdUser {
                definition: config_home
                    .join("systemd")
                    .join("user")
                    .join(crate::NODE_SYSTEMD_SERVICE),
            });
        }
        #[cfg(target_os = "windows")]
        {
            let _ = user_home;
            return Ok(Self::Windows);
        }
        #[allow(unreachable_code)]
        Err(super::NodeNativeServiceError::UnsupportedPlatform)
    }

    pub(super) fn manager_name(&self) -> &'static str {
        match self {
            Self::Launchd { .. } => "launchd",
            Self::SystemdUser { .. } => "systemd-user",
            Self::Windows => "windows-service",
        }
    }
}

#[derive(Clone)]
pub(super) struct CommandSpec {
    pub(super) program: &'static str,
    pub(super) args: Vec<OsString>,
}

impl CommandSpec {
    pub(super) fn new(program: &'static str) -> Self {
        Self {
            program,
            args: Vec::new(),
        }
    }

    pub(super) fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<OsString>,
    {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }
}
