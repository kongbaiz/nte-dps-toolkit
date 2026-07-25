#![windows_subsystem = "windows"]

fn main() {
    let log_path = nte_dps_tool::platform::update_install::updater_log_path_from_args();
    match nte_dps_tool::platform::update_install::run_from_args() {
        Ok(()) => {
            if let Some(path) = log_path {
                nte_dps_tool::platform::update_install::append_updater_log(
                    &path,
                    "update completed",
                );
            }
        }
        Err(error) => {
            if let Some(path) = log_path {
                nte_dps_tool::platform::update_install::append_updater_log(
                    &path,
                    &format!("update failed: {error}"),
                );
            }
            std::process::exit(1);
        }
    }
}
