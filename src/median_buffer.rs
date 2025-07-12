use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct RunningMedianBuffer {
    buffer: VecDeque<f32>,
    capacity: usize,
}

impl RunningMedianBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, value: f32) {
        if self.buffer.len() >= self.capacity {
            self.buffer.pop_front();
        }
        self.buffer.push_back(value);
    }

    pub fn median(&self) -> Option<f32> {
        if self.buffer.is_empty() {
            return None;
        }

        let mut sorted: Vec<f32> = self.buffer.iter().copied().collect();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

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
        self.buffer.len() >= self.capacity
    }
}

#[derive(Debug, Clone)]
pub struct RunningMedianBufferU16 {
    buffer: VecDeque<u16>,
    capacity: usize,
}

impl RunningMedianBufferU16 {
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: VecDeque::with_capacity(capacity),
            capacity,
        }
    }

    pub fn push(&mut self, value: u16) {
        if self.buffer.len() >= self.capacity {
            self.buffer.pop_front();
        }
        self.buffer.push_back(value);
    }

    pub fn median(&self) -> Option<u16> {
        if self.buffer.is_empty() {
            return None;
        }

        let mut sorted: Vec<u16> = self.buffer.iter().copied().collect();
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
        self.buffer.len() >= self.capacity
    }
}

