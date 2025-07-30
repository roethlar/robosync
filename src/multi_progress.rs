//! Multi-line progress display for parallel workers

use std::sync::{Arc, Mutex};
use std::time::Instant;
use std::io::{self, Write};
use crossterm::{cursor, terminal, QueueableCommand};

/// Worker status for multi-line display
#[derive(Clone)]
pub struct WorkerStatus {
    pub name: String,
    pub total_items: u64,
    pub completed_items: u64,
    pub total_bytes: u64,
    pub completed_bytes: u64,
    pub start_time: Instant,
    pub status: WorkerState,
}

#[derive(Clone, PartialEq)]
pub enum WorkerState {
    Pending,
    Running,
    Completed,
    Failed(String),
}

/// Multi-line progress display for parallel execution
pub struct MultiProgress {
    workers: Arc<Mutex<Vec<WorkerStatus>>>,
    start_time: Instant,
    last_update: Arc<Mutex<Instant>>,
    update_interval: std::time::Duration,
    lines_used: Arc<Mutex<usize>>,
}

impl MultiProgress {
    pub fn new() -> Self {
        Self {
            workers: Arc::new(Mutex::new(Vec::new())),
            start_time: Instant::now(),
            last_update: Arc::new(Mutex::new(Instant::now())),
            update_interval: std::time::Duration::from_millis(100),
            lines_used: Arc::new(Mutex::new(0)),
        }
    }

    /// Register a new worker
    pub fn add_worker(&self, name: String, total_items: u64, total_bytes: u64) -> usize {
        let mut workers = self.workers.lock().unwrap();
        let id = workers.len();
        workers.push(WorkerStatus {
            name,
            total_items,
            completed_items: 0,
            total_bytes,
            completed_bytes: 0,
            start_time: Instant::now(),
            status: WorkerState::Pending,
        });
        id
    }

    /// Update worker progress
    pub fn update_worker(&self, id: usize, completed_items: u64, completed_bytes: u64) {
        if let Ok(mut workers) = self.workers.lock() {
            if let Some(worker) = workers.get_mut(id) {
                worker.completed_items = completed_items;
                worker.completed_bytes = completed_bytes;
                if worker.status == WorkerState::Pending {
                    worker.status = WorkerState::Running;
                }
            }
        }
        self.maybe_refresh();
    }

    /// Mark worker as completed
    pub fn complete_worker(&self, id: usize) {
        if let Ok(mut workers) = self.workers.lock() {
            if let Some(worker) = workers.get_mut(id) {
                worker.status = WorkerState::Completed;
            }
        }
        self.maybe_refresh();
    }

    /// Mark worker as failed
    pub fn fail_worker(&self, id: usize, error: String) {
        if let Ok(mut workers) = self.workers.lock() {
            if let Some(worker) = workers.get_mut(id) {
                worker.status = WorkerState::Failed(error);
            }
        }
        self.maybe_refresh();
    }

    /// Check if we should refresh the display
    fn maybe_refresh(&self) {
        let now = Instant::now();
        let should_update = {
            let last = self.last_update.lock().unwrap();
            now.duration_since(*last) >= self.update_interval
        };
        
        if should_update {
            *self.last_update.lock().unwrap() = now;
            self.refresh_display();
        }
    }

    /// Refresh the multi-line display
    pub fn refresh_display(&self) {
        let workers = self.workers.lock().unwrap();
        let mut stdout = io::stdout();
        
        // Check if we're in a terminal
        let is_terminal = atty::is(atty::Stream::Stdout);
        
        // Clear previous lines only if in terminal
        if is_terminal {
            if let Ok(lines) = self.lines_used.lock() {
                for _ in 0..*lines {
                    let _ = stdout.queue(cursor::MoveUp(1));
                    let _ = stdout.queue(terminal::Clear(terminal::ClearType::CurrentLine));
                }
            }
        }
        
        // Calculate totals
        let mut total_items = 0u64;
        let mut completed_items = 0u64;
        let mut total_bytes = 0u64;
        let mut completed_bytes = 0u64;
        
        for worker in workers.iter() {
            total_items += worker.total_items;
            completed_items += worker.completed_items;
            total_bytes += worker.total_bytes;
            completed_bytes += worker.completed_bytes;
        }
        
        let elapsed = self.start_time.elapsed();
        let throughput = if elapsed.as_secs() > 0 {
            completed_bytes / elapsed.as_secs()
        } else {
            completed_bytes
        };
        
        // Print overall progress
        println!("{}/{} files | {} | {}/s | {:.1}s",
            format_number(completed_items),
            format_number(total_items),
            format_bytes(completed_bytes),
            format_bytes(throughput),
            elapsed.as_secs_f32()
        );
        
        // Print worker status
        for worker in workers.iter() {
            let icon = match &worker.status {
                WorkerState::Pending => "⏳",
                WorkerState::Running => "⚡",
                WorkerState::Completed => "✅",
                WorkerState::Failed(_) => "❌",
            };
            
            let progress = if worker.total_items > 0 {
                (worker.completed_items as f32 / worker.total_items as f32 * 100.0) as u32
            } else {
                0
            };
            
            let worker_throughput = if worker.start_time.elapsed().as_secs() > 0 {
                worker.completed_bytes / worker.start_time.elapsed().as_secs()
            } else {
                worker.completed_bytes
            };
            
            println!("{} {}: {}/{} ({:>3}%) | {} @ {}/s",
                icon,
                worker.name,
                format_number(worker.completed_items),
                format_number(worker.total_items),
                progress,
                format_bytes(worker.completed_bytes),
                format_bytes(worker_throughput)
            );
        }
        
        *self.lines_used.lock().unwrap() = workers.len() + 1;
        let _ = stdout.flush();
    }

    /// Final summary after all workers complete
    pub fn finish(&self) {
        self.refresh_display();
        println!(); // Extra newline after final display
        
        let workers = self.workers.lock().unwrap();
        let elapsed = self.start_time.elapsed();
        
        println!("\nWorker Summary:");
        for worker in workers.iter() {
            let duration = worker.start_time.elapsed();
            match &worker.status {
                WorkerState::Completed => {
                    let throughput = if duration.as_secs() > 0 {
                        worker.completed_bytes / duration.as_secs()
                    } else {
                        worker.completed_bytes
                    };
                    println!("  ✅ {}: {} files in {:.1}s ({} at {}/s)",
                        worker.name,
                        format_number(worker.completed_items),
                        duration.as_secs_f32(),
                        format_bytes(worker.completed_bytes),
                        format_bytes(throughput)
                    );
                }
                WorkerState::Failed(err) => {
                    println!("  ❌ {}: Failed after {:.1}s - {}",
                        worker.name,
                        duration.as_secs_f32(),
                        err
                    );
                }
                _ => {}
            }
        }
    }
}

/// Format bytes into human readable string
fn format_bytes(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let exponent = (bytes as f64).log(1024.0).floor() as usize;
    let exponent = exponent.min(UNITS.len() - 1);
    let value = bytes as f64 / 1024_f64.powi(exponent as i32);
    if exponent == 0 {
        format!("{} {}", bytes, UNITS[exponent])
    } else {
        format!("{:.1} {}", value, UNITS[exponent])
    }
}

/// Format number with thousands separators
fn format_number(n: u64) -> String {
    let s = n.to_string();
    let chars: Vec<char> = s.chars().collect();
    let mut result = String::new();
    
    for (i, &ch) in chars.iter().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            result.push(',');
        }
        result.push(ch);
    }
    
    result.chars().rev().collect()
}