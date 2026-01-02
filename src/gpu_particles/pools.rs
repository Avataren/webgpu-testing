// src/gpu_particles/pools.rs

use std::cell::RefCell;
use std::marker::PhantomData;
use std::ops::{Deref, DerefMut};
use std::rc::Rc;

use crate::gpu_particles::particle::Particle;

/// RAII guard that returns pooled object on drop
pub struct PooledVec<T> {
    vec: Option<Vec<T>>,
    // Callback invoked on drop to return the vec to the pool. Using a boxed
    // FnMut avoids storing raw pointers to the thread-local pools.
    return_callback: Option<Box<dyn FnMut(Vec<T>)>>,
    _not_send_sync: PhantomData<Rc<RefCell<VecPool<T>>>>,
}

impl<T> PooledVec<T> {
    pub fn clear(&mut self) {
        if let Some(vec) = &mut self.vec {
            vec.clear();
        }
    }
}

impl<T> Deref for PooledVec<T> {
    type Target = Vec<T>;
    fn deref(&self) -> &Self::Target {
        self.vec.as_ref().unwrap()
    }
}

impl<T> DerefMut for PooledVec<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.vec.as_mut().unwrap()
    }
}

impl<T> Drop for PooledVec<T> {
    fn drop(&mut self) {
        if let Some(mut vec) = self.vec.take() {
            vec.clear();
            if let Some(cb) = self.return_callback.as_mut() {
                cb(vec);
            }
        }
    }
}

/// Pool for Vec<T> objects
pub struct VecPool<T> {
    pool: Vec<Vec<T>>,
    default_capacity: usize,
}

impl<T> VecPool<T> {
    pub fn new(default_capacity: usize) -> Self {
        Self {
            pool: Vec::new(),
            default_capacity,
        }
    }

    pub fn with_preallocated(count: usize, capacity: usize) -> Self {
        let mut pool = Vec::with_capacity(count);
        for _ in 0..count {
            pool.push(Vec::with_capacity(capacity));
        }
        Self {
            pool,
            default_capacity: capacity,
        }
    }

    pub fn acquire(&mut self) -> Vec<T> {
        let mut vec = self
            .pool
            .pop()
            .unwrap_or_else(|| Vec::with_capacity(self.default_capacity));
        vec.clear();
        vec
    }

    fn return_vec(&mut self, mut vec: Vec<T>) {
        vec.clear();
        if vec.capacity() <= self.default_capacity * 4 {
            self.pool.push(vec);
        }
    }

    pub fn shrink_to(&mut self, max_size: usize) {
        if self.pool.len() > max_size {
            self.pool.truncate(max_size);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vec_pool_basic() {
        let mut pool = VecPool::<u32>::new(10);

        let mut vec1 = pool.acquire();
        vec1.push(1);
        vec1.push(2);

        pool.return_vec(vec1);

        let vec2 = pool.acquire();
        assert_eq!(vec2.len(), 0);
        assert!(vec2.capacity() >= 10);
    }
}

thread_local! {
    static PARTICLE_VEC_POOL: RefCell<VecPool<Particle>> = RefCell::new(
        VecPool::with_preallocated(4, 1024)
    );

    static U32_VEC_POOL: RefCell<VecPool<u32>> = RefCell::new(
        VecPool::with_preallocated(4, 256)
    );

    static SPAWN_REQUEST_POOL: RefCell<VecPool<(u32, Particle)>> = RefCell::new(
        VecPool::with_preallocated(2, 256)
    );
}

/// Acquire a pooled vector of particles.
pub fn acquire_particle_vec() -> PooledVec<Particle> {
    PARTICLE_VEC_POOL.with(|pool| {
        let vec = pool.borrow_mut().acquire();
        // Build a small callback that returns the vec into the thread-local pool.
        let cb = Box::new(|vec: Vec<Particle>| {
            PARTICLE_VEC_POOL.with(|pool| pool.borrow_mut().return_vec(vec));
        });

        PooledVec {
            vec: Some(vec),
            return_callback: Some(cb),
            _not_send_sync: PhantomData,
        }
    })
}

/// Acquire a pooled vector of u32.
pub fn acquire_u32_vec() -> PooledVec<u32> {
    U32_VEC_POOL.with(|pool| {
        let vec = pool.borrow_mut().acquire();
        let cb = Box::new(|vec: Vec<u32>| {
            U32_VEC_POOL.with(|pool| pool.borrow_mut().return_vec(vec));
        });

        PooledVec {
            vec: Some(vec),
            return_callback: Some(cb),
            _not_send_sync: PhantomData,
        }
    })
}

/// Acquire a pooled vector of spawn requests.
pub fn acquire_spawn_request_vec() -> PooledVec<(u32, Particle)> {
    SPAWN_REQUEST_POOL.with(|pool| {
        let vec = pool.borrow_mut().acquire();
        let cb = Box::new(|vec: Vec<(u32, Particle)>| {
            SPAWN_REQUEST_POOL.with(|pool| pool.borrow_mut().return_vec(vec));
        });

        PooledVec {
            vec: Some(vec),
            return_callback: Some(cb),
            _not_send_sync: PhantomData,
        }
    })
}

/// Periodically clean up pools
pub fn maintain_pools() {
    PARTICLE_VEC_POOL.with(|pool| pool.borrow_mut().shrink_to(4));
    U32_VEC_POOL.with(|pool| pool.borrow_mut().shrink_to(4));
    SPAWN_REQUEST_POOL.with(|pool| pool.borrow_mut().shrink_to(2));
}
