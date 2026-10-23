use crate::{Dom, DomHandle, listen};
use day_spec::{NodeId, Proposal, Size, controls::searchfield::*};
fn make(toolkit: &mut Dom, props: &SearchProps, _id: NodeId) -> DomHandle {
    let h = toolkit.element("input");
    toolkit.set_attr(&h, "type", "search");
    toolkit.set_attr(&h, "value", &props.text);
    toolkit.set_attr(&h, "placeholder", &props.placeholder);
    toolkit.listen(&h, listen::INPUT);
    h
}
fn update(toolkit: &mut Dom, h: &DomHandle, patch: &SearchPatch) {
    let SearchPatch::SetText(text) = patch;
    toolkit.set_attr(h, "value", text);
}
fn measure(_toolkit: &mut Dom, _h: &DomHandle, proposal: Proposal) -> Size {
    Size::new(proposal.width.unwrap_or(180.0), 28.0)
}
day_pieces::dom_renderer!(crate::register_renderer, Dom,
    kind: KIND, props: SearchProps, patch: SearchPatch,
    make: make, update: update, measure: measure);
