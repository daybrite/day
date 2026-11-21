// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Run with a display (on headless Linux: xvfb-run cargo test -p day-toolkit-gtk --test `native_surface_tint`).
use day_spec::{
    Color, ImageSource, NodeId, Shadow, Toolkit, kinds,
    props::{ContainerProps, ImagePatch, ImageProps},
};
use day_toolkit_gtk::Gtk;
use gtk4::prelude::*;
use std::sync::Arc;

/// A 4×2 PNG: the left half red, the right half blue.
const RED_BLUE: [u8; 76] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x04, 0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00, 0x00, 0xf0, 0xca, 0xea,
    0x34, 0x00, 0x00, 0x00, 0x13, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0xf8, 0xcf, 0xc0, 0x00,
    0x44, 0x60, 0xe2, 0x3f, 0x03, 0x32, 0x07, 0x00, 0x67, 0xb2, 0x07, 0xf9, 0xf7, 0x92, 0x56, 0x82,
    0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// The picture's pixels as premultiplied `0xAARRGGBB`, row by row.
fn pixels(picture: &gtk4::Widget) -> Vec<u32> {
    let texture = picture
        .downcast_ref::<gtk4::Picture>()
        .and_then(gtk4::Picture::paintable)
        .and_then(|p| p.downcast::<gtk4::gdk::Texture>().ok())
        .expect("the image shows a texture");
    let stride = texture.width() as usize * 4;
    let mut data = vec![0u8; stride * texture.height() as usize];
    texture.download(&mut data, stride);
    data.as_chunks::<4>()
        .0
        .iter()
        .map(|p| u32::from_ne_bytes(*p))
        .collect()
}

fn main() {
    gtk4::init().expect("native surface and tint test needs a display");
    let mut toolkit = Gtk::new();

    // A tinted raster from bytes: every pixel the tint, the (opaque) alpha kept.
    let bytes = ImageSource::Bytes(Arc::new(RED_BLUE.to_vec()));
    let image = toolkit.realize(
        kinds::IMAGE,
        &ImageProps {
            source: bytes.clone(),
            tint: Some(Color::rgb(0.0, 1.0, 0.0).into()),
            ..Default::default()
        },
        NodeId(51),
    );
    assert_eq!(pixels(&image), [0xff00_ff00; 8]);
    // A new tint repaints from the recorded source.
    toolkit.update(
        &image,
        kinds::IMAGE,
        &ImagePatch::Tint(Some(Color::rgb(0.0, 0.0, 1.0).into())),
        None,
    );
    assert_eq!(pixels(&image), [0xff00_00ff; 8]);
    // No tint: the authored colors again.
    toolkit.update(&image, kinds::IMAGE, &ImagePatch::Tint(None), None);
    assert_eq!(pixels(&image)[0], 0xffff_0000);
    assert_eq!(pixels(&image)[3], 0xff00_00ff);
    // A source swap keeps the tint that is on the view.
    toolkit.update(
        &image,
        kinds::IMAGE,
        &ImagePatch::Tint(Some(Color::rgb(1.0, 1.0, 0.0).into())),
        None,
    );
    toolkit.update(&image, kinds::IMAGE, &ImagePatch::Source(bytes), None);
    assert_eq!(pixels(&image), [0xffff_ff00; 8]);
    toolkit.release(image);

    // A shadowed surface does not clip, and neither do the plain containers it is put in:
    // a GtkFixed clips its children by default, which would cut the shadow at the stack's edge.
    let plain = |toolkit: &mut Gtk, id| {
        toolkit.realize(kinds::CONTAINER, &ContainerProps::default(), NodeId(id))
    };
    let (outer, stack) = (plain(&mut toolkit, 60), plain(&mut toolkit, 61));
    let clipping = toolkit.realize(
        kinds::CONTAINER,
        &ContainerProps {
            corner_radius: 8.0,
            clips: true,
            ..Default::default()
        },
        NodeId(62),
    );
    let card = toolkit.realize(
        kinds::CONTAINER,
        &ContainerProps {
            fill: Some(Color::WHITE.into()),
            corner_radius: 12.0,
            shadow: Some(Shadow::new(Color::BLACK, 8.0).offset(0.0, 4.0)),
            ..Default::default()
        },
        NodeId(63),
    );
    assert_eq!(stack.overflow(), gtk4::Overflow::Hidden);
    assert_eq!(card.overflow(), gtk4::Overflow::Visible);
    // The stack is mounted before the card is: the walk goes up from wherever a shadow lands.
    toolkit.insert(&clipping, &outer, 0);
    toolkit.insert(&outer, &stack, 0);
    toolkit.insert(&stack, &card, 0);
    assert_eq!(stack.overflow(), gtk4::Overflow::Visible);
    assert_eq!(outer.overflow(), gtk4::Overflow::Visible);
    // A container that was asked to clip still does.
    assert_eq!(clipping.overflow(), gtk4::Overflow::Hidden);
    // And one mounted after it holds the card carries the walk on.
    let (late, later) = (plain(&mut toolkit, 64), plain(&mut toolkit, 65));
    let second = toolkit.realize(
        kinds::CONTAINER,
        &ContainerProps {
            shadow: Some(Shadow::new(Color::BLACK, 2.0)),
            ..Default::default()
        },
        NodeId(66),
    );
    toolkit.insert(&late, &second, 0);
    assert_eq!(late.overflow(), gtk4::Overflow::Visible);
    assert_eq!(later.overflow(), gtk4::Overflow::Hidden);
    toolkit.insert(&later, &late, 0);
    assert_eq!(later.overflow(), gtk4::Overflow::Visible);
    println!("native surface and tint: raster tint patches and shadow spill passed");
}
