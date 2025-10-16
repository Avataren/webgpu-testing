#![allow(dead_code)]
// src/gpu_particles/system/slot_allocator_v2.rs

/// High-performance slot allocator using bitsets for O(1) allocation
pub struct SlotAllocator {
    occupied: Vec<u64>,
    high_water: u32,
    first_free_hint: u32,
    capacity: u32,
    occupied_count: u32,
}

impl SlotAllocator {
    const BITS_PER_WORD: u32 = 64;

    pub fn new(capacity: u32) -> Self {
        let words_needed = capacity.div_ceil(Self::BITS_PER_WORD) as usize;
        Self {
            occupied: vec![0; words_needed],
            high_water: 0,
            first_free_hint: 0,
            capacity,
            occupied_count: 0,
        }
    }

    #[inline]
    pub fn allocate(&mut self) -> Option<u32> {
        if self.occupied_count >= self.capacity {
            return None;
        }

        // Fast path: try hint first
        if self.first_free_hint < self.capacity && !self.is_occupied(self.first_free_hint) {
            let slot = self.first_free_hint;
            self.set_occupied(slot);
            self.update_after_allocate(slot);
            return Some(slot);
        }

        // Slow path: scan for free slot
        let start_word = (self.first_free_hint / Self::BITS_PER_WORD) as usize;
        for word_idx in start_word..self.occupied.len() {
            if self.occupied[word_idx] != u64::MAX {
                let bit_idx = self.occupied[word_idx].trailing_ones();
                let slot = word_idx as u32 * Self::BITS_PER_WORD + bit_idx;
                if slot < self.capacity {
                    self.set_occupied(slot);
                    self.update_after_allocate(slot);
                    return Some(slot);
                }
            }
        }

        None
    }

    #[inline]
    pub fn reclaim_batch(&mut self, slots: &[u32]) {
        for &slot in slots {
            self.reclaim(slot);
        }
        self.compact_trailing_free_slots();
    }

    #[inline]
    pub fn reclaim(&mut self, slot: u32) -> bool {
        if slot >= self.capacity || !self.is_occupied(slot) {
            return false;
        }

        self.set_free(slot);
        self.occupied_count -= 1;

        if slot < self.first_free_hint {
            self.first_free_hint = slot;
        }

        true
    }

    pub fn compact_trailing_free_slots(&mut self) {
        while self.high_water > 0 && !self.is_occupied(self.high_water - 1) {
            self.high_water -= 1;
        }
    }

    pub fn initialize_with_count(&mut self, count: u32) {
        let count = count.min(self.capacity);

        for word in &mut self.occupied {
            *word = 0;
        }

        let full_words = (count / Self::BITS_PER_WORD) as usize;
        for word in &mut self.occupied[..full_words] {
            *word = u64::MAX;
        }

        let remaining_bits = count % Self::BITS_PER_WORD;
        if remaining_bits > 0 && full_words < self.occupied.len() {
            self.occupied[full_words] = (1u64 << remaining_bits) - 1;
        }

        self.high_water = count;
        self.occupied_count = count;
        self.first_free_hint = count;
    }

    #[inline]
    pub fn high_water(&self) -> u32 {
        self.high_water
    }

    #[inline]
    fn is_occupied(&self, slot: u32) -> bool {
        let word_idx = (slot / Self::BITS_PER_WORD) as usize;
        let bit_idx = slot % Self::BITS_PER_WORD;
        (self.occupied[word_idx] & (1u64 << bit_idx)) != 0
    }

    #[inline]
    fn set_occupied(&mut self, slot: u32) {
        let word_idx = (slot / Self::BITS_PER_WORD) as usize;
        let bit_idx = slot % Self::BITS_PER_WORD;
        self.occupied[word_idx] |= 1u64 << bit_idx;
    }

    #[inline]
    fn set_free(&mut self, slot: u32) {
        let word_idx = (slot / Self::BITS_PER_WORD) as usize;
        let bit_idx = slot % Self::BITS_PER_WORD;
        self.occupied[word_idx] &= !(1u64 << bit_idx);
    }

    #[inline]
    fn update_after_allocate(&mut self, slot: u32) {
        self.occupied_count += 1;

        if slot >= self.high_water {
            self.high_water = slot + 1;
        }

        if slot == self.first_free_hint {
            self.first_free_hint = self.find_first_free_after(slot);
        }
    }

    fn find_first_free_after(&self, start: u32) -> u32 {
        for slot in (start + 1)..self.capacity {
            if !self.is_occupied(slot) {
                return slot;
            }
        }
        self.capacity
    }
}

impl Default for SlotAllocator {
    fn default() -> Self {
        Self::new(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_basic_allocation() {
        let mut allocator = SlotAllocator::new(100);

        let slot1 = allocator.allocate().unwrap();
        let slot2 = allocator.allocate().unwrap();
        let slot3 = allocator.allocate().unwrap();

        assert_eq!(slot1, 0);
        assert_eq!(slot2, 1);
        assert_eq!(slot3, 2);
    }

    #[test]
    fn test_reclaim_and_reuse() {
        let mut allocator = SlotAllocator::new(100);

        let _slot1 = allocator.allocate().unwrap();
        let slot2 = allocator.allocate().unwrap();
        allocator.allocate().unwrap();

        allocator.reclaim(slot2);

        let slot4 = allocator.allocate().unwrap();
        assert_eq!(slot4, slot2);
    }
    #[test]
    fn test_batch_reclaim() {
        let mut allocator = SlotAllocator::new(100);

        let mut slots = Vec::new();
        for _ in 0..20 {
            slots.push(allocator.allocate().unwrap());
        }

        let to_reclaim = vec![5, 10, 15, 19];
        allocator.reclaim_batch(&to_reclaim);

        assert_eq!(allocator.high_water(), 19);
    }
}
