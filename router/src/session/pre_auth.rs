//! Pre-auth connection pool (C-session §4): an independent total cap equal to
//! `runtime.maxConcurrency` by default. The permit is held from accept until
//! the registered ACK is written (or any terminal/timeout/disconnect).

use std::collections::HashSet;

#[derive(Debug)]
pub struct PreAuthPool {
    limit: usize,
    occupied: HashSet<String>,
    refused: u64,
}

impl PreAuthPool {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            occupied: HashSet::new(),
            refused: 0,
        }
    }

    pub fn try_acquire(&mut self, connection_id: &str) -> bool {
        if self.occupied.len() >= self.limit {
            self.refused += 1;
            return false;
        }
        self.occupied.insert(connection_id.to_string())
    }

    pub fn release(&mut self, connection_id: &str) {
        self.occupied.remove(connection_id);
    }

    pub fn limit(&self) -> usize {
        self.limit
    }

    pub fn occupied(&self) -> usize {
        self.occupied.len()
    }

    pub fn refused(&self) -> u64 {
        self.refused
    }
}
