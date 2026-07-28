#[cfg(not(unix))]
compile_error!("Flashy Factory v1 supports Unix-like operating systems only");

#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[cfg(test)]
pub(crate) struct TestEnvGuard {
    name: &'static str,
    previous: Option<std::ffi::OsString>,
}

#[cfg(test)]
impl TestEnvGuard {
    pub(crate) fn set(name: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(name);
        unsafe { std::env::set_var(name, value) };
        Self { name, previous }
    }
}

#[cfg(test)]
impl Drop for TestEnvGuard {
    fn drop(&mut self) {
        unsafe {
            if let Some(value) = self.previous.take() {
                std::env::set_var(self.name, value);
            } else {
                std::env::remove_var(self.name);
            }
        }
    }
}

pub mod approval;
pub mod clone;
pub mod config;
pub mod daemon;
pub mod execution;
pub mod fleet;
pub mod forge;
pub mod github;
mod hash;
pub mod init;
pub mod inspection;
pub mod repository;
pub mod runtime;
pub mod sandbox;
pub mod source;
pub mod storage;
mod table;
pub mod workflow;
pub mod workspace;
