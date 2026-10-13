//! Where each frame's time goes, by stage (scripts, transforms, render, UI, ...), for
//! `engine_stats`. Always on: one clock read per stage per frame.

use std::cell::RefCell;

/// Weight of the newest frame in the running average.
const SMOOTHING: f64 = 0.05;

#[derive(Default)]
struct Timings {
    /// Stages timed so far this frame, in first-seen order, in milliseconds.
    current: Vec<(&'static str, f64)>,
    /// The previous complete frame.
    last: Vec<(&'static str, f64)>,
    /// Exponential moving average per stage.
    average: Vec<(&'static str, f64)>,
}

thread_local! {
    static TIMINGS: RefCell<Timings> = RefCell::default();
}

fn add(list: &mut Vec<(&'static str, f64)>, stage: &'static str, ms: f64) {
    match list.iter_mut().find(|(name, _)| *name == stage) {
        Some((_, total)) => *total += ms,
        None => list.push((stage, ms)),
    }
}

/// Run `f` and add its duration to `stage` for this frame. Nested stages are counted
/// in both, so time them side by side rather than inside each other.
pub fn time<R>(stage: &'static str, f: impl FnOnce() -> R) -> R {
    let start = instant::Instant::now();
    let result = f();
    let ms = start.elapsed().as_secs_f64() * 1000.0;
    TIMINGS.with(|timings| add(&mut timings.borrow_mut().current, stage, ms));
    result
}

/// Close the frame: its stages become the "last frame" and feed the averages.
pub fn end_frame() {
    TIMINGS.with(|timings| {
        let timings = &mut *timings.borrow_mut();
        let frame = std::mem::take(&mut timings.current);
        for (stage, average) in &mut timings.average {
            let ms = frame
                .iter()
                .find(|(name, _)| name == stage)
                .map_or(0.0, |(_, ms)| *ms);
            *average += (ms - *average) * SMOOTHING;
        }
        for (stage, ms) in &frame {
            if !timings.average.iter().any(|(name, _)| name == stage) {
                timings.average.push((stage, *ms));
            }
        }
        timings.last = frame;
    });
}

/// `(stage, last frame ms, average ms)` for every stage seen, in frame order.
pub fn snapshot() -> Vec<(&'static str, f64, f64)> {
    TIMINGS.with(|timings| {
        let timings = timings.borrow();
        timings
            .average
            .iter()
            .map(|(stage, average)| {
                let last = timings
                    .last
                    .iter()
                    .find(|(name, _)| name == stage)
                    .map_or(0.0, |(_, ms)| *ms);
                (*stage, last, *average)
            })
            .collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stages_accumulate_per_frame_and_average() {
        time("a", || {});
        time("b", || {
            std::thread::sleep(std::time::Duration::from_millis(2))
        });
        time("b", || {});
        end_frame();
        let stats = snapshot();
        assert_eq!(
            stats.iter().map(|(stage, ..)| *stage).collect::<Vec<_>>(),
            ["a", "b"]
        );
        let (_, last_b, average_b) = stats[1];
        assert!(last_b >= 2.0, "{last_b}");
        assert_eq!(last_b, average_b, "the first frame seeds the average");
        end_frame();
        let (_, last_b, average_b) = snapshot()[1];
        assert_eq!(
            last_b, 0.0,
            "a stage that did not run this frame took no time"
        );
        assert!(average_b < 2.0 * (1.0 - SMOOTHING) + 1.0);
    }
}
