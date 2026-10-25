// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Native clipboard probe: write/read encoded bytes without starting a Day app.
use day_part_clipboard::{ClipboardFuture, Content, Representation};
fn native<T>(mut future: ClipboardFuture<T>) -> T {
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    match future.as_mut().poll(&mut cx) {
        std::task::Poll::Ready(r) => r.expect("clipboard access"),
        _ => panic!("use a browser event loop on web"),
    }
}
fn main() {
    let args: Vec<_> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("write") => {
            let data = std::fs::read(&args[3]).unwrap();
            println!(
                "{:?}",
                native(day_part_clipboard::write(Content(vec![
                    Representation::new(&args[2], data)
                ])))
            );
        }
        Some("read") => {
            let data = native(day_part_clipboard::read(&[&args[2]])).expect("representation");
            std::fs::write(&args[3], &*data.bytes).unwrap();
            println!("{} bytes", data.bytes.len());
        }
        Some("roundtrip") => {
            let bytes = std::fs::read(&args[2]).unwrap();
            let expected = Content(vec![
                Representation::new("image/png", bytes.clone()),
                Representation::new("application/x-day-test", vec![0, 255, 0, 128]),
                Representation::new("text/plain", b"clipboard test".to_vec()),
            ]);
            assert_eq!(native(day_part_clipboard::write(expected.clone())).len(), 3);
            for r in expected.0 {
                assert_eq!(native(day_part_clipboard::read(&[&r.mime])).unwrap(), r);
            }
            println!("PNG, binary NUL/non-UTF8, and text representations round-tripped");
        }
        _ => panic!("write/read MIME PATH | roundtrip PNG_PATH"),
    }
}
