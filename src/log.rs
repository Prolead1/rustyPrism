use std::fmt::Arguments;
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub enum LogLevel {
    Info,
    Warning,
    Error,
    Debug,
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

    pub fn log(&self, level: LogLevel, args: Arguments) {
        let elapsed = self.start_time.elapsed();
        let formatted_time = format_time(elapsed);

        let log_level_str = match level {
            LogLevel::Info => "INFO",
            LogLevel::Warning => "WARNING",
            LogLevel::Error => "ERROR",
            LogLevel::Debug => "DEBUG",
        };

        let mut locked_output = self.output.lock().unwrap();
        writeln!(
            &mut *locked_output,
            "[{}] [{}] - {}",
            log_level_str, formatted_time, args
        )
        .expect("Failed to write log message");
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
        crate::log::LOGGER.log(crate::log::LogLevel::Info, format_args!($($arg)*))
    };
}

macro_rules! log_warn {
    ($($arg:tt)*) => {
        crate::log::LOGGER.log(crate::log::LogLevel::Warning, format_args!($($arg)*))
    };
}

macro_rules! log_error {
    ($($arg:tt)*) => {
        crate::log::LOGGER.log(crate::log::LogLevel::Error, format_args!($($arg)*))
    };
}

macro_rules! log_debug {
    ($($arg:tt)*) => {
        crate::log::LOGGER.log(crate::log::LogLevel::Debug, format_args!($($arg)*))
    };
}

#[test]
fn test_logger() {
    log_debug!("This is an informational message: {}", 42);
    log_warn!("This is a warning message: {}", "something");
    log_error!("This is an error message: {}", true);
}
