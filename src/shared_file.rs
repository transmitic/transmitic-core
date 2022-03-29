use std::error::Error;

use serde::{Serialize, Deserialize};

use crate::config::file_contains_only_valid_chars;

#[derive(Debug, Clone)]
pub struct SelectedDownload {
    pub path: String,
    pub owner: String,
}

#[derive(Debug)]
pub struct RefreshData {
    pub owner: String,
    pub data: Result<SharedFile, Box<dyn Error>>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct SharedFile {
    pub path: String,
    pub is_directory: bool,
    pub files: Vec<SharedFile>,
    pub file_size: u64,
    pub size_string: String,
}

impl SharedFile {
    pub fn new(path: String, is_directory: bool, files: Vec<SharedFile>, file_size: u64) -> Self {
        let size_string = get_file_size_string(file_size);
        return SharedFile {
            path: path,
            is_directory: is_directory,
            files: files,
            file_size: file_size,
            size_string: size_string,
        }
    }

    // TODO added because I couldn't easily get big ints into the UI with the existing structure.
    //  so create the string on the backend
    pub fn set_file_size_string(&mut self) {
        self.size_string = get_file_size_string(self.file_size);
    }
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
    let mut ftype = "file";
    if shared_file.is_directory {
        ftype = "dir";
    }

    println!(
        "{}{} | ({}) ({})",
        spacer, shared_file.path, shared_file.size_string, ftype
    );
    if shared_file.is_directory {
        let mut new_spacer = spacer.clone();
        new_spacer.push_str("    ");
        for sub_file in &shared_file.files {
            print_shared_files(&sub_file, &new_spacer);
        }
    }
}

fn get_file_size_string(mut bytes: u64) -> String {
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