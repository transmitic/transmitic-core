use serde::{Deserialize, Serialize, Serializer};

use crate::config::file_contains_only_valid_chars;

#[derive(Debug, Clone)]
pub struct SelectedDownload {
    pub path: String,
    pub owner: String,
    pub file_size: u64,
}

#[derive(Debug, Clone)]
pub struct RefreshData {
    pub owner: String,
    pub error: Option<String>,
    pub data: Option<SharedFile>,
    pub in_progress: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SharedFile {
    pub path: String,
    pub is_directory: bool,
    pub files: Vec<SharedFile>,
    #[serde(serialize_with = "u64_to_string")] // Needed for UI
    #[serde(deserialize_with = "string_to_u64")] // Needed for UI
    pub file_size: u64,
    pub size_string: String,
}

fn u64_to_string<S>(x: &u64, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    s.serialize_str(&x.to_string())
}

fn string_to_u64<'de, T, D>(de: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: std::str::FromStr,
    <T as std::str::FromStr>::Err: std::fmt::Display,
{
    String::deserialize(de)?
        .parse()
        .map_err(serde::de::Error::custom)
}

impl SharedFile {
    pub fn new(path: String, is_directory: bool, files: Vec<SharedFile>, file_size: u64) -> Self {
        let size_string = get_file_size_string(file_size);
        SharedFile {
            path,
            is_directory,
            files,
            file_size,
            size_string,
        }
    }

    // TODO added because I couldn't easily get big ints into the UI with the existing structure.
    //  so create the string on the backend
    pub fn set_file_size_string(&mut self) {
        self.size_string = get_file_size_string(self.file_size);
    }
}

/// Reset the file size strings to ensure they are accurate to prevent malicious mismatches from server
pub fn reset_file_size_string(shared_file: &mut SharedFile) {
    if !shared_file.is_directory {
        shared_file.set_file_size_string();
        return;
    }

    for file in shared_file.files.iter_mut() {
        if file.is_directory {
            reset_file_size_string(file);
        }
        file.set_file_size_string();
    }
    shared_file.set_file_size_string();
}

pub fn remove_invalid_files(shared_file: &mut SharedFile) {
    if shared_file.is_directory {
        shared_file
            .files
            .retain(|x| file_contains_only_valid_chars(&x.path));
        for s in shared_file.files.iter_mut() {
            remove_invalid_files(s);
        }
    }
}

pub fn print_shared_files(shared_file: &SharedFile, spacer: &str) {
    let mut ftype = "file";
    if shared_file.is_directory {
        ftype = "dir";
    }

    println!(
        "{}{} | ({}) ({})",
        spacer, shared_file.path, shared_file.size_string, ftype
    );
    if shared_file.is_directory {
        let mut new_spacer = spacer.to_string();
        new_spacer.push_str("    ");
        for sub_file in &shared_file.files {
            print_shared_files(sub_file, &new_spacer);
        }
    }
}

fn get_file_size_string(mut bytes: u64) -> String {
    let gig: u64 = 1_073_741_824;
    let meg: u64 = 1_048_576;
    let kb: u64 = 1024;

    let divisor: u64;
    let unit: String;

    if bytes == 0 {
        bytes = 1;
    }

    if bytes >= gig {
        divisor = gig;
        unit = "GB".to_string();
    } else if bytes >= meg {
        divisor = meg;
        unit = "MB".to_string();
    } else if bytes >= kb {
        divisor = kb;
        unit = "KB".to_string();
    } else {
        divisor = 1;
        unit = "bytes".to_string();
    }

    let size = bytes as f64 / divisor as f64;

    let size_string: String = if unit == "bytes" {
        format!("{} {}", size, &unit)
    } else {
        format!("{:.2} {}", size, &unit)
    };

    size_string
}
