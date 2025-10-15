#[derive(Default)]
pub(crate) struct ParticleSlotAllocator {
    next_slot: u32,
    free_slots: Vec<u32>,
}

impl ParticleSlotAllocator {
    pub(crate) fn allocate(&mut self, max_particles: u32) -> Option<u32> {
        if let Some(slot) = self.free_slots.pop() {
            Some(slot)
        } else if self.next_slot < max_particles {
            let slot = self.next_slot;
            self.next_slot += 1;
            Some(slot)
        } else {
            None
        }
    }

    pub(crate) fn reclaim(&mut self, index: u32, max_particles: u32) -> bool {
        if index >= max_particles || index >= self.next_slot {
            return false;
        }

        if self.free_slots.contains(&index) {
            return false;
        }

        self.free_slots.push(index);
        true
    }

    pub(crate) fn compact_trailing_free_slots(&mut self) {
        while self.next_slot > 0 {
            let tail_index = self.next_slot - 1;
            if let Some(pos) = self.free_slots.iter().position(|&slot| slot == tail_index) {
                self.free_slots.swap_remove(pos);
                self.next_slot -= 1;
            } else {
                break;
            }
        }
    }

    pub(crate) fn initialize_with_count(&mut self, count: u32) {
        self.next_slot = count;
        self.free_slots.clear();
    }

    pub(crate) fn high_water(&self) -> u32 {
        self.next_slot
    }
}
