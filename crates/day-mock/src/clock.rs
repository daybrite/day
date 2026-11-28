// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

use std::cell::RefCell;
use std::collections::BTreeMap;

type Job = Box<dyn FnOnce() + Send>;

// Three ticks per millisecond represents both millisecond sleeps and 60 Hz frames exactly.
const TICKS_PER_MS: u64 = 3;
const FRAME_TICKS: u64 = 50;

#[derive(Default)]
struct Clock {
    now: u64,
    sequence: u64,
    timers: BTreeMap<(u64, u64), Job>,
    frame_due: Option<u64>,
}

thread_local! {
    static CLOCK: RefCell<Option<Clock>> = const { RefCell::new(None) };
}

pub(super) fn reset(manual: bool) {
    day_reactive::set_virtual_timers(manual);
    CLOCK.with(|clock| clock.replace(manual.then(Clock::default)));
}

pub(super) fn enabled() -> bool {
    CLOCK.with(|clock| clock.borrow().is_some())
}

pub(super) fn schedule(ms: u32, job: Job) {
    CLOCK.with_borrow_mut(|clock| {
        let clock = clock.as_mut().unwrap();
        let key = (clock.now + u64::from(ms) * TICKS_PER_MS, clock.sequence);
        clock.sequence += 1;
        clock.timers.insert(key, job);
    });
}

pub(super) fn request_frame() {
    CLOCK.with_borrow_mut(|clock| {
        if let Some(clock) = clock {
            clock.frame_due = Some((clock.now / FRAME_TICKS + 1) * FRAME_TICKS);
        }
    });
}

enum Event {
    Timer(Job),
    Frame,
}

pub(super) fn advance(ms: u32) {
    let end = CLOCK.with_borrow(|clock| clock.as_ref().unwrap().now + u64::from(ms) * TICKS_PER_MS);
    let mut last = None;
    let mut at_same_time = 0;
    loop {
        // Release the queue before dispatch: waking a task can arm another timer, including
        // one due inside this advance. A nested advance may already have passed our end.
        let next = CLOCK.with_borrow_mut(|clock| {
            let clock = clock.as_mut()?;
            if clock.now > end {
                return None;
            }
            let timer = clock.timers.first_key_value().map(|(&(due, _), _)| due);
            let deadline = clock.frame_due.into_iter().chain(timer).min();
            let due = deadline.filter(|&due| due <= end);
            clock.now = due.unwrap_or(end);
            let seconds = clock.now as f64 / 3000.0;
            super::MOCK_CLOCK.with(|c| c.set(seconds));
            due.map(|due| {
                let event = if clock.frame_due == Some(due) {
                    clock.frame_due = None;
                    Event::Frame
                } else {
                    Event::Timer(clock.timers.pop_first().unwrap().1)
                };
                (due, seconds, event)
            })
        });
        let Some((due, seconds, event)) = next else {
            break;
        };
        if last != Some(due) {
            at_same_time = 0;
            last = Some(due);
        }
        at_same_time += 1;
        assert!(
            at_same_time <= 100_000,
            "mock clock cannot advance: 100,000 callbacks at the same instant"
        );
        super::viewport_flush();
        match event {
            Event::Timer(job) => job(),
            Event::Frame => {
                let due = super::FRAMES.with(|f| std::mem::take(&mut *f.borrow_mut()));
                for (_, callback) in due {
                    callback(day_spec::FrameStamp::new(seconds));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{MockHandle, MockToolkit};
    use day_spec::{Platform, Toolkit};
    use std::cell::Cell;
    use std::rc::Rc;

    #[test]
    fn partial_advances_share_one_frame_grid_and_new_mocks_discard_old_timers() {
        let (mut mock, probe) = MockToolkit::new_with_manual_clock();
        let fired = Rc::new(Cell::new(0.0));
        let stamp = fired.clone();
        let _cancel = mock.request_frame(&MockHandle(0), Box::new(move |f| stamp.set(f.timestamp)));
        probe.advance(8);
        probe.advance(8);
        assert_eq!(fired.get(), 0.0);
        probe.advance(1);
        assert!((fired.get() - 1.0 / 60.0).abs() < f64::EPSILON);

        MockToolkit::post_delayed(5, Box::new(|| panic!("old mock timer fired")));
        let (_, fresh) = MockToolkit::new_with_manual_clock();
        fresh.advance(5);
        assert!((fresh.now() - 0.005).abs() < f64::EPSILON);
    }
}
