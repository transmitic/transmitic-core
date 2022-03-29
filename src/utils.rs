use std::error::Error;

use crate::shared_file::SharedFile;


// TODO remove this module

pub fn get_file_by_path(file_choice: &str, shared_file: &SharedFile) -> Option<SharedFile> {
	if shared_file.path == file_choice {
		return Some(shared_file.clone());
	}

	for a_file in &shared_file.files {
		if let Some(found) =  get_file_by_path(file_choice, &a_file) {
			return Some(found.clone());
		}
	}

	return None;
}

