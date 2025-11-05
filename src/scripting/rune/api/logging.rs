use log::{debug, error, info, warn};
use rune::runtime::VmResult;

#[rune::function]
pub(crate) fn log_debug(message: String) -> VmResult<()> {
    debug!(target: "script", "{message}");
    VmResult::Ok(())
}

#[rune::function]
pub(crate) fn log_info(message: String) -> VmResult<()> {
    info!(target: "script", "{message}");
    VmResult::Ok(())
}

#[rune::function]
pub(crate) fn log_warn(message: String) -> VmResult<()> {
    warn!(target: "script", "{message}");
    VmResult::Ok(())
}

#[rune::function]
pub(crate) fn log_error(message: String) -> VmResult<()> {
    error!(target: "script", "{message}");
    VmResult::Ok(())
}
