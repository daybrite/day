// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
use super::*;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;
const KIND: Sensor = SensorKind::Accelerometer.with_calibration(Calibration::Calibrated);
const READING: SensorReading = SensorReading {
    x: 1.0,
    y: 2.0,
    z: 3.0,
};
fn recv<T>(rx: &Receiver<T>) -> T {
    rx.recv_timeout(Duration::from_secs(3))
        .expect("dispatcher event")
}
struct Source(mpsc::Sender<&'static str>);
impl Drop for Source {
    fn drop(&mut self) {
        self.0.send("stop").unwrap();
    }
}

#[test]
fn event_delivery_shares_source_and_last_drop_stops_it() {
    let hub = Arc::new(Hub::default());
    let (samples, rx) = mpsc::channel();
    let tx = samples.clone();
    let (a, _) = hub.add(
        KIND,
        Box::new(move |r| {
            tx.send(r).unwrap();
        }),
    );
    let (b, _) = hub.add(
        KIND,
        Box::new(move |r| {
            samples.send(r).unwrap();
        }),
    );
    let (events, lifecycle) = mpsc::channel();
    let h = hub.clone();
    let worker = std::thread::spawn(move || {
        h.run(|_, _| {
            events.send("start").unwrap();
            Some(Source(events.clone()))
        })
    });
    assert_eq!(recv(&lifecycle), "start");
    assert!(
        rx.try_recv().is_err(),
        "subscription alone must not invent a reading"
    );
    hub.push(KIND, 1, READING);
    assert_eq!(recv(&rx), READING);
    assert_eq!(recv(&rx), READING);
    drop(a);
    hub.push(KIND, 1, READING);
    assert_eq!(recv(&rx), READING);
    assert!(lifecycle.try_recv().is_err(), "one watcher remains");
    drop(b);
    assert_eq!(recv(&lifecycle), "stop");
    worker.join().unwrap();
    assert!(rx.try_recv().is_err(), "no periodic repeated samples");
}

#[test]
fn pending_samples_coalesce_and_stale_epochs_are_rejected() {
    let hub = Arc::new(Hub::default());
    let (old, _) = hub.add(KIND, Box::new(|_| panic!("old watcher")));
    hub.push(KIND, 1, READING);
    drop(old);
    let (samples, rx) = mpsc::channel();
    let (watch, _) = hub.add(
        KIND,
        Box::new(move |r| {
            samples.send(r).unwrap();
        }),
    );
    hub.push(KIND, 1, READING);
    assert!(lock(&hub.state).feeds[0].pending.is_none());
    hub.push(KIND, 2, READING);
    let newest = SensorReading { x: 4.0, ..READING };
    hub.push(KIND, 2, newest);
    hub.push(
        KIND,
        2,
        SensorReading {
            x: f64::NAN,
            ..READING
        },
    );
    let h = hub.clone();
    let worker = std::thread::spawn(move || h.run(|_, _| Some(())));
    assert_eq!(recv(&rx), newest);
    drop(watch);
    worker.join().unwrap();
    assert!(rx.try_recv().is_err());
}

#[test]
fn callback_can_drop_itself_and_subscribe_again() {
    let hub = Arc::new(Hub::default());
    let own = Arc::new(Mutex::new(None));
    let replacement = Arc::new(Mutex::new(None));
    let (done, rx) = mpsc::channel();
    let h = hub.clone();
    let o = own.clone();
    let r = replacement.clone();
    let (watch, _) = hub.add(
        KIND,
        Box::new(move |_| {
            lock(&o).take();
            *lock(&r) = Some(h.add(KIND, Box::new(|_| {})).0);
            done.send(()).unwrap();
        }),
    );
    *lock(&own) = Some(watch);
    let (events, lifecycle) = mpsc::channel();
    let h = hub.clone();
    let worker = std::thread::spawn(move || {
        h.run(|_, _| {
            events.send("start").unwrap();
            Some(Source(events.clone()))
        })
    });
    assert_eq!(recv(&lifecycle), "start");
    hub.push(KIND, 1, READING);
    recv(&rx);
    assert_eq!(recv(&lifecycle), "stop");
    assert_eq!(recv(&lifecycle), "start");
    lock(&replacement).take();
    assert_eq!(recv(&lifecycle), "stop");
    worker.join().unwrap();
}

#[test]
fn panic_in_one_callback_does_not_stop_other_watchers_or_teardown() {
    let hub = Arc::new(Hub::default());
    let (a, _) = hub.add(KIND, Box::new(|_| panic!("synthetic callback failure")));
    let (tx, rx) = mpsc::channel();
    let (b, _) = hub.add(
        KIND,
        Box::new(move |_| {
            tx.send(()).unwrap();
        }),
    );
    let h = hub.clone();
    let worker = std::thread::spawn(move || h.run(|_, _| Some(())));
    hub.push(KIND, 1, READING);
    recv(&rx);
    hub.push(KIND, 1, READING);
    recv(&rx);
    drop(a);
    drop(b);
    worker.join().unwrap();
}

#[test]
fn reentrant_delivery_is_coalesced_without_locking_user_code() {
    let listener = Arc::new(Listener {
        active: AtomicBool::new(true),
        delivery: Mutex::new(Delivery {
            handler: None,
            pending: None,
        }),
    });
    let weak = Arc::downgrade(&listener);
    let (tx, rx) = mpsc::channel();
    lock(&listener.delivery).handler = Some(Box::new(move |r| {
        tx.send(r.x).unwrap();
        if r.x == 1.0 {
            let l = weak.upgrade().unwrap();
            l.deliver(SensorReading { x: 2.0, ..r });
            l.deliver(SensorReading { x: 3.0, ..r });
        }
    }));
    listener.deliver(READING);
    assert_eq!(recv(&rx), 1.0);
    assert_eq!(recv(&rx), 3.0);
    assert!(rx.try_recv().is_err());
    assert!(Sensor::from_code(6).is_none());
}

#[test]
fn drop_during_start_releases_old_source_before_restarting() {
    let hub = Arc::new(Hub::default());
    let (old, _) = hub.add(KIND, Box::new(|_| panic!("cancelled watcher")));
    let (started, start_rx) = mpsc::channel();
    let (resume, resume_rx) = mpsc::channel();
    let (events, lifecycle) = mpsc::channel();
    let h = hub.clone();
    let worker = std::thread::spawn(move || {
        h.run(|_, generation| {
            started.send(generation).unwrap();
            if generation == 1 {
                recv(&resume_rx);
            }
            Some(Source(events.clone()))
        })
    });
    assert_eq!(recv(&start_rx), 1);
    drop(old);
    let (samples, rx) = mpsc::channel();
    let (new, _) = hub.add(
        KIND,
        Box::new(move |r| {
            samples.send(r).unwrap();
        }),
    );
    hub.push(KIND, 1, READING);
    resume.send(()).unwrap();
    assert_eq!(recv(&lifecycle), "stop");
    assert_eq!(recv(&start_rx), 2);
    hub.push(KIND, 2, SensorReading { x: 2.0, ..READING });
    assert_eq!(recv(&rx).x, 2.0);
    drop(new);
    assert_eq!(recv(&lifecycle), "stop");
    worker.join().unwrap();
}

#[test]
fn calibration_modes_have_independent_sources_delivery_and_lifetimes() {
    let hub = Arc::new(Hub::default());
    let calibrated = SensorKind::Magnetometer.with_calibration(Calibration::Calibrated);
    let raw = SensorKind::Magnetometer.with_calibration(Calibration::Uncalibrated);
    let (samples, rx) = mpsc::channel();
    let tx = samples.clone();
    let (a, _) = hub.add(calibrated, Box::new(move |r| tx.send((true, r)).unwrap()));
    let (b, _) = hub.add(raw, Box::new(move |r| samples.send((false, r)).unwrap()));
    let (starts, started) = mpsc::channel();
    let (stops, stopped) = mpsc::channel();
    let h = hub.clone();
    let worker = std::thread::spawn(move || {
        h.run(|sensor, _| {
            starts.send(sensor).unwrap();
            Some(Source(stops.clone()))
        })
    });
    assert_eq!(recv(&started), calibrated);
    assert_eq!(recv(&started), raw);
    hub.push(calibrated, 1, READING);
    assert_eq!(recv(&rx), (true, READING));
    let biased = SensorReading {
        x: 150.0,
        ..READING
    };
    hub.push(raw, 1, biased);
    assert_eq!(recv(&rx), (false, biased));
    drop(a);
    assert_eq!(recv(&stopped), "stop");
    hub.push(calibrated, 1, READING); // cancelled mode must not leak into the other mode
    hub.push(raw, 1, biased);
    assert_eq!(recv(&rx), (false, biased));
    assert!(rx.try_recv().is_err());
    assert!(stopped.try_recv().is_err());
    drop(b);
    assert_eq!(recv(&stopped), "stop");
    worker.join().unwrap();
}
