// Progress display module that handles spinner and status updates
use indicatif::{ProgressBar, ProgressStyle};
use std::sync::Arc;
use std::time::{Duration, Instant};
use std::io::{self, Write};
use crossterm::style::{Color, Stylize};

pub struct ProgressDisplay {
    pub spinner: ProgressBar,
    start_time: Instant,
}

impl ProgressDisplay {
    pub fn new() -> Arc<Self> {
        // Force immediate output on Windows console
        #[cfg(windows)]
        {
            // Flush stdout to ensure any pending output is displayed
            let _ = io::stdout().flush();
            
            // Enable virtual terminal processing for ANSI codes on Windows
            #[cfg(windows)]
            unsafe {
                use winapi::um::consoleapi::{GetConsoleMode, SetConsoleMode};
                use winapi::um::processenv::GetStdHandle;
                use winapi::um::winbase::STD_OUTPUT_HANDLE;
                
                let handle = GetStdHandle(STD_OUTPUT_HANDLE);
                if handle != winapi::um::handleapi::INVALID_HANDLE_VALUE {
                    let mut mode: u32 = 0;
                    if GetConsoleMode(handle, &mut mode) != 0 {
                        // Enable ENABLE_VIRTUAL_TERMINAL_PROCESSING (0x0004)
                        let _ = SetConsoleMode(handle, mode | 0x0004);
                    }
                }
            }
        }
        
        let spinner = ProgressBar::new_spinner();
        
        // Use simpler spinner chars that work better on Windows
        let spinner_style = if cfg!(windows) {
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg}")
                .unwrap_or_else(|_| ProgressStyle::default_spinner())
                .tick_chars("⣾⣽⣻⢿⡿⣟⣯⣷")
        } else {
            ProgressStyle::default_spinner()
                .template("{spinner:.green} {msg}")
                .unwrap_or_else(|_| ProgressStyle::default_spinner())
        };
        
        spinner.set_style(spinner_style);
        spinner.set_message("[00:00.0] Scanning files...");
        
        // Start ticking immediately with faster rate for visibility
        spinner.enable_steady_tick(Duration::from_millis(80));
        
        // Force initial draw
        spinner.tick();
        
        Arc::new(Self {
            spinner,
            start_time: Instant::now(),
        })
    }
    
    pub fn update(&self, files: u64, bytes: u64, errors: u64) {
        self.update_with_warnings(files, bytes, 0, errors);
    }
    
    pub fn update_with_warnings(&self, files: u64, bytes: u64, warnings: u64, errors: u64) {
        let elapsed = self.start_time.elapsed();
        let mut msg = format!(
            "[{:02}:{:02}.{:01}] Discovered: {} files | Transferred: {:.2} MB",
            elapsed.as_secs() / 60,
            elapsed.as_secs() % 60,
            (elapsed.as_millis() % 1000) / 100,
            files,
            bytes as f64 / 1_048_576.0
        );
        
        // Only show warnings if > 0, in yellow
        if warnings > 0 {
            msg.push_str(&format!(" | {}", format!("Warnings: {}", warnings).with(Color::Yellow)));
        }
        
        // Only show errors if > 0, in red  
        if errors > 0 {
            msg.push_str(&format!(" | {}", format!("Errors: {}", errors).with(Color::Red)));
        }
        
        self.spinner.set_message(msg);
    }
    
    /// New method that shows discovered vs copied files clearly
    pub fn update_detailed(&self, discovered: u64, copied: u64, bytes: u64, warnings: u64, errors: u64) {
        let elapsed = self.start_time.elapsed();
        let mut msg = format!(
            "[{:02}:{:02}.{:01}] Found: {} | Copied: {} files ({:.2} MB)",
            elapsed.as_secs() / 60,
            elapsed.as_secs() % 60,
            (elapsed.as_millis() % 1000) / 100,
            discovered,
            copied,
            bytes as f64 / 1_048_576.0
        );
        
        // Calculate and show rates if time has passed
        let elapsed_secs = elapsed.as_secs_f64();
        if elapsed_secs > 1.0 {
            let discovery_rate = discovered as f64 / elapsed_secs;
            let transfer_rate = bytes as f64 / elapsed_secs / 1_048_576.0; // MB/s
            msg.push_str(&format!(" | {:.0} files/s, {:.1} MB/s", discovery_rate, transfer_rate));
        }
        
        // Only show warnings if > 0, in yellow
        if warnings > 0 {
            msg.push_str(&format!(" | {}", format!("Warn: {}", warnings).with(Color::Yellow)));
        }
        
        // Only show errors if > 0, in red  
        if errors > 0 {
            msg.push_str(&format!(" | {}", format!("Err: {}", errors).with(Color::Red)));
        }
        
        self.spinner.set_message(msg);
    }
    
    pub fn set_message(&self, msg: impl Into<String>) {
        let elapsed = self.start_time.elapsed();
        let timed_msg = format!(
            "[{:02}:{:02}.{:01}] {}",
            elapsed.as_secs() / 60,
            elapsed.as_secs() % 60,
            (elapsed.as_millis() % 1000) / 100,
            msg.into()
        );
        self.spinner.set_message(timed_msg);
    }
    
    pub fn finish(&self) {
        self.spinner.finish_with_message("Done");
    }
    
    pub fn finish_and_clear(&self) {
        self.spinner.finish_and_clear();
    }
}