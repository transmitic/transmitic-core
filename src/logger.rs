use std::{
    collections::VecDeque,
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    time::SystemTime,
};

pub static LOG_MESSAGES_COUNT: usize = 1000;
static LOG_FILE_ROTATE_COUNT: usize = LOG_MESSAGES_COUNT * 2;

#[cfg(debug_assertions)]
pub static DEFAULT_LOG_LEVEL: LogLevel = LogLevel::Debug;

#[cfg(not(debug_assertions))]
pub static DEFAULT_LOG_LEVEL: LogLevel = LogLevel::Warning;

#[cfg(debug_assertions)]
pub static DEFAULT_LOG_TO_FILE: bool = true;

#[cfg(not(debug_assertions))]
pub static DEFAULT_LOG_TO_FILE: bool = false;

#[derive(Clone, Debug, Copy)]
pub enum LogLevel {
    Critical = 50,
    Error = 40,
    Warning = 30,
    Info = 20,
    Debug = 10,
}
pub struct Logger {
    log_lines: VecDeque<String>,
    active_log_level: LogLevel,
    is_log_to_file: bool,
    log_path: PathBuf,
    log_file_write_count: usize,
}

impl Logger {
    pub fn new(log_level: LogLevel, is_log_to_file: bool) -> Logger {
        let mut log_path = env::current_exe().unwrap();
        log_path.pop();
        log_path.push("transmitic_log.txt");
        fs::remove_file(&log_path).ok();

        Logger {
            log_lines: VecDeque::with_capacity(1000),
            active_log_level: log_level,
            is_log_to_file,
            log_path,
            log_file_write_count: 0,
        }
    }

    pub fn get_log_path(&self) -> PathBuf {
        self.log_path.clone()
    }

    pub fn set_log_level(&mut self, log_level: LogLevel) {
        self.active_log_level = log_level;
    }

    pub fn get_log_level(&self) -> LogLevel {
        self.active_log_level
    }

    pub fn set_is_file_logging(&mut self, state: bool) {
        self.is_log_to_file = state;
    }

    pub fn get_is_file_logging(&self) -> bool {
        self.is_log_to_file
    }

    pub fn get_log_messages(&self) -> Vec<String> {
        let logs: Vec<String> = self.log_lines.clone().into();
        logs
    }

    pub fn log_message(&mut self, log_level: LogLevel, message: String) {
        if (log_level as u8) < (self.active_log_level as u8) {
            return;
        }

        let time_string = match SystemTime::now().duration_since(SystemTime::UNIX_EPOCH) {
            Ok(n) => n.as_secs().to_string(),
            Err(_) => "TimeError".to_string(),
        };

        // TODO duped strings with UI
        let log_string = match log_level {
            LogLevel::Critical => "[CRITICAL]",
            LogLevel::Error =>    "[ERROR]   ",
            LogLevel::Warning =>  "[WARNING] ",
            LogLevel::Info =>     "[INFO]    ",
            LogLevel::Debug =>    "[DEBUG]   ",
        };

        let message = format!("{} {} |  {}", log_string, time_string, message);
        println!("{}", message);

        while self.log_lines.len() >= LOG_MESSAGES_COUNT {
            self.log_lines.pop_back();
        }
        self.log_lines.push_front(message.clone());

        if self.is_log_to_file {
            self.log_to_file(message);
        }
    }

    fn log_to_file(&mut self, message: String) {
        if self.log_file_write_count >= LOG_FILE_ROTATE_COUNT {
            match OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&self.log_path)
            {
                Ok(mut f) => {
                    for message in self.log_lines.iter() {
                        let _ = f.write(format!("{}\n", message).as_bytes()).ok();
                    }
                }
                Err(e) => {
                    self.log_lines
                        .push_front(format!("[ERROR] | Failed to open truncate log {}", e));
                }
            }

            self.log_file_write_count = 0;
        } else {
            match OpenOptions::new()
                .create(true)
                .append(true)
                .open(&self.log_path)
            {
                Ok(mut f) => {
                    let _ = f.write(format!("{}\n", message).as_bytes()).ok();
                }
                Err(e) => {
                    self.log_lines
                        .push_front(format!("[ERROR] | Failed to open append log {}", e));
                }
            }

            self.log_file_write_count += 1;
        }
    }
}
