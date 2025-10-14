#[cfg(feature = "profile")]
use once_cell::sync::Lazy;
#[cfg(feature = "profile")]
use std::fs::{File, OpenOptions};
#[cfg(feature = "profile")]
use std::io::Write;
#[cfg(feature = "profile")]
use std::sync::Mutex;
#[cfg(feature = "profile")]
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Only compile this section when the "profile" feature is enabled
#[cfg(feature = "profile")]
mod chrome_profiler {

    mod fxhash {
        use std::hash::{Hash, Hasher};

        pub fn hash32<T: Hash>(t: &T) -> u32 {
            use std::collections::hash_map::DefaultHasher;
            let mut s = DefaultHasher::new();
            t.hash(&mut s);
            (s.finish() & 0xFFFF_FFFF) as u32
        }
    }

    use super::*;

    static TRACE_FILE: Lazy<Mutex<File>> = Lazy::new(|| {
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open("trace.json")
            .unwrap();

        // Start Chrome trace JSON
        writeln!(file, "{{\"traceEvents\":[").unwrap();
        Mutex::new(file)
    });

    pub struct ChromeProfiler {
        name: &'static str,
        start: Instant,
    }

    impl ChromeProfiler {
        #[inline]
        pub fn new(name: &'static str) -> Self {
            Self {
                name,
                start: Instant::now(),
            }
        }
    }

    impl Drop for ChromeProfiler {
        fn drop(&mut self) {
            let dur = self.start.elapsed().as_micros() as u64;
            let start_us = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_micros() as u64
                - dur;

            // Workaround: make a numeric hash from ThreadId (stable)
            let tid = format!("{:?}", std::thread::current().id());
            let tid_hash = fxhash::hash32(&tid);

            let event = format!(
                "{{\"name\":\"{}\",\"ph\":\"X\",\"ts\":{},\"dur\":{},\"pid\":1,\"tid\":{}}},\n",
                self.name, start_us, dur, tid_hash
            );

            let mut file = TRACE_FILE.lock().unwrap();
            file.write_all(event.as_bytes()).unwrap();
        }
    }

    impl Drop for FileEndMarker {
        fn drop(&mut self) {
            let mut file = TRACE_FILE.lock().unwrap();
            writeln!(file, "{{}}]}}").unwrap();
        }
    }

    #[allow(dead_code)]
    struct FileEndMarker;
    static _CLOSE_TRACE: Lazy<FileEndMarker> = Lazy::new(|| FileEndMarker);

    pub fn begin_profile_scope(name: &'static str) -> ChromeProfiler {
        ChromeProfiler::new(name)
    }
}

/// Stub no-op profiler if the feature is disabled
#[cfg(not(feature = "profile"))]
mod chrome_profiler {
    pub struct ChromeProfiler;
    #[inline(always)]
    pub fn begin_profile_scope(_name: &'static str) -> ChromeProfiler {
        ChromeProfiler
    }
}

pub use chrome_profiler::begin_profile_scope;

/// Macro for easily profiling a function or scope.
/// Automatically uses the current function name.
#[macro_export]
macro_rules! profile_scope {
    () => {
        #[cfg(feature = "profile")]
        let _profiler = $crate::profiler::begin_profile_scope(std::any::type_name::<fn()>());
    };
    ($name:expr) => {
        #[cfg(feature = "profile")]
        let _profiler = $crate::profiler::begin_profile_scope($name);
    };
}
