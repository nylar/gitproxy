use std::path::PathBuf;

use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct Config {
    #[serde(default = "default_port")]
    pub port: u16,
    pub root_dir: PathBuf,
    pub git_user_name: String,
    pub git_user_email: String,
}

const fn default_port() -> u16 {
    3000
}
