use core::cmp::Ordering;
use defmt::Format;
use heapless::{Deque, Vec};

#[derive(Debug, Clone, Format)]
pub struct RunningMedianBuffer<const CAPACITY: usize> {
    buffer: Deque<f32, CAPACITY>,
}

impl<const CAPACITY: usize> RunningMedianBuffer<CAPACITY> {
    pub fn new() -> Self {
        Self {
            buffer: Deque::new(),
        }
    }

    pub fn push(&mut self, value: f32) {
        if self.buffer.len() >= CAPACITY {
            let _ = self.buffer.pop_front();
        }
        let _ = self.buffer.push_back(value);
    }

    pub fn median(&self) -> Option<f32> {
        if self.buffer.is_empty() {
            return None;
        }

        let mut sorted: Vec<f32, CAPACITY> = Vec::new();
        for &value in self.buffer.iter() {
            let _ = sorted.push(value);
        }
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));

        let len = sorted.len();
        if len % 2 == 0 {
            Some((sorted[len / 2 - 1] + sorted[len / 2]) / 2.0)
        } else {
            Some(sorted[len / 2])
        }
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn is_full(&self) -> bool {
        self.buffer.len() >= CAPACITY
    }
}

#[derive(Debug, Clone)]
pub struct RunningMedianBufferU16<const CAPACITY: usize> {
    buffer: Deque<u16, CAPACITY>,
}

impl<const CAPACITY: usize> RunningMedianBufferU16<CAPACITY> {
    pub fn new() -> Self {
        Self {
            buffer: Deque::new(),
        }
    }

    pub fn push(&mut self, value: u16) {
        if self.buffer.len() >= CAPACITY {
            let _ = self.buffer.pop_front();
        }
        let _ = self.buffer.push_back(value);
    }

    pub fn median(&self) -> Option<u16> {
        if self.buffer.is_empty() {
            return None;
        }

        let mut sorted: Vec<u16, CAPACITY> = Vec::new();
        for &value in self.buffer.iter() {
            let _ = sorted.push(value);
        }
        sorted.sort();

        let len = sorted.len();
        if len % 2 == 0 {
            Some((sorted[len / 2 - 1] + sorted[len / 2]) / 2)
        } else {
            Some(sorted[len / 2])
        }
    }

    pub fn clear(&mut self) {
        self.buffer.clear();
    }

    pub fn len(&self) -> usize {
        self.buffer.len()
    }

    pub fn is_full(&self) -> bool {
        self.buffer.len() >= CAPACITY
    }
}
