use std::env;
use std::fmt::Arguments;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(PartialEq, Debug, Clone, Copy)]
pub enum LogLevel {
    Debug = 0,
    Info = 1,
    Warn = 2,
    Error = 3,
}

pub struct Logger<T: Write> {
    output: Arc<Mutex<T>>,
    start_time: Instant,
}

impl<T: Write> Logger<T> {
    fn new(output: T) -> Self {
        Logger {
            output: Arc::new(Mutex::new(output)),
            start_time: Instant::now(),
        }
    }

    pub fn log(&self, level: LogLevel, args: Arguments, module: &'static str) {
        let log_level = get_log_level();
        let module = module.splitn(2, "::").nth(1).unwrap_or(module);

        if level as u8 >= log_level as u8 {
            let elapsed = self.start_time.elapsed();
            let formatted_time = format_time(elapsed);

            let log_level_str = match level {
                LogLevel::Info => format!("{:^5}", "INFO"),
                LogLevel::Warn => format!("{:^5}", "WARN"),
                LogLevel::Error => format!("{:^5}", "ERROR"),
                LogLevel::Debug => format!("{:^5}", "DEBUG"),
            };

            let mut locked_output = self.output.lock().unwrap();
            writeln!(
                &mut *locked_output,
                "{} [{}] {} - {}",
                formatted_time, log_level_str, module, args
            )
            .expect("Failed to write log message");
        }
    }
}
fn format_time(duration: Duration) -> String {
    let total_milliseconds = duration.as_millis();
    let milliseconds = total_milliseconds % 1000;
    let total_seconds = total_milliseconds / 1000;
    let seconds = total_seconds % 60;
    let total_minutes = total_seconds / 60;
    let minutes = total_minutes % 60;
    let hours = total_minutes / 60;

    format!(
        "{:02}:{:02}:{:02}.{:03}",
        hours, minutes, seconds, milliseconds
    )
}
lazy_static::lazy_static! {
    pub static ref LOGGER: Logger<std::io::Stdout> = Logger::new(std::io::stdout());
}

macro_rules! log_info {
    ($($arg:tt)*) => {
        crate::log::LOGGER.log(crate::log::LogLevel::Info, format_args!($($arg)*), module_path!())
    };
}

macro_rules! log_warn {
    ($($arg:tt)*) => {
        crate::log::LOGGER.log(crate::log::LogLevel::Warn, format_args!($($arg)*), module_path!())
    };
}

macro_rules! log_error {
    ($($arg:tt)*) => {
        crate::log::LOGGER.log(crate::log::LogLevel::Error, format_args!($($arg)*), module_path!())
    };
}

macro_rules! log_debug {
    ($($arg:tt)*) => {
        crate::log::LOGGER.log(crate::log::LogLevel::Debug, format_args!($($arg)*), module_path!())
    };
}

fn get_log_level() -> LogLevel {
    match env::var("APP_LOGLEVEL") {
        Ok(value) => match value.to_lowercase().as_str() {
            "info" => LogLevel::Info,
            "warn" => LogLevel::Warn,
            "error" => LogLevel::Error,
            "debug" => LogLevel::Debug,
            _ => LogLevel::Info, // Default log level if the value is invalid
        },
        Err(_) => LogLevel::Info, // Default log level if the environment variable is not set
    }
}

#[test]
fn test_logger() {
    env::set_var("APP_LOGLEVEL", "debug");
    log_debug!("This is an informational message: {}", 42);
    log_warn!("This is a warn message: {}", "something");
    log_error!("This is an error message: {}", true);
}
