use std::path::PathBuf;

use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct Config {
    #[serde(default = "default_port")]
    pub port: u16,
    pub root_dir: PathBuf,
}

const fn default_port() -> u16 {
    3000
}
