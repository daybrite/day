// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
use crate::res;
use day::prelude::*;

#[rustfmt::skip]
pub fn volume_controls() -> impl Piece {
    let volume = Signal::new(0.4);
    column((
        label(res::str::volume())
            .a11y(|a| a.role(Role::Heading(2))),
        slider(volume).id(id!(volume))
            .a11y(move |a| a.label(res::str::volume())
                .hint(res::str::volume_hint())
                .value(res::str::percentage(volume))),
        button(res::str::reset()).action(move || {
            volume.set(0.4);
            announce(&res::str::volume_reset().format());
        }),
    )).padding(Space::M)
}
