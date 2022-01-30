use serde::{Serialize, Deserialize};

use crate::config::file_contains_only_valid_chars;

#[derive(Debug, Clone)]
pub struct SelectedDownload {
    pub path: String,
    pub owner: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SharedFile {
    pub path: String,
    pub is_directory: bool,
    pub files: Vec<SharedFile>,
    pub file_size: u64,
}

pub fn remove_invalid_files(shared_file: &mut SharedFile) {
    if shared_file.is_directory {
        shared_file.files.retain(|x|file_contains_only_valid_chars(x));

        for s in shared_file.files.iter_mut() {
            remove_invalid_files(s);
        }
    }
}

pub fn print_shared_files(shared_file: &SharedFile, spacer: &String) {
    let file_size_string = get_file_size_string(shared_file.file_size);

    let mut ftype = "file";
    if shared_file.is_directory {
        ftype = "dir";
    }

    println!(
        "{}{} | ({}) ({})",
        spacer, shared_file.path, file_size_string, ftype
    );
    if shared_file.is_directory {
        let mut new_spacer = spacer.clone();
        new_spacer.push_str("    ");
        for sub_file in &shared_file.files {
            print_shared_files(&sub_file, &new_spacer);
        }
    }
}

pub fn get_file_size_string(mut bytes: u64) -> String {
    let gig: u64 = 1_000_000_000;
    let meg: u64 = 1_000_000;
    let byte: u64 = 1000;

    let divisor: u64;
    let unit: String;

    if bytes == 0 {
        bytes = 1;
        divisor = 1;
        unit = "b".to_string();
    } else if bytes >= gig {
        divisor = gig;
        unit = "GB".to_string();
    } else if bytes >= meg {
        divisor = meg;
        unit = "MB".to_string();
    } else if bytes >= byte {
        divisor = byte;
        unit = "KB".to_string();
    } else {
        divisor = byte;
        unit = "b".to_string();
    }

    let size = bytes as f64 / divisor as f64;
    let mut size_string = String::from(format!("{:.2}", size));
    size_string.push_str(" ");
    size_string.push_str(&unit);
    return size_string;
}