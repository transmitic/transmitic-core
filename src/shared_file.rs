use serde::{Serialize, Deserialize};


#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SharedFile {
    pub path: String,
    pub is_directory: bool,
    pub files: Vec<SharedFile>,
    pub file_size: u64,
}