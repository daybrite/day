use crate::{Dom, DomHandle, listen};
use day_spec::{
    NodeId, Proposal, Size,
    controls::combobox::{ComboPatch, ComboProps, KIND},
};
fn make(toolkit: &mut Dom, props: &ComboProps, _id: NodeId) -> DomHandle {
    let h = toolkit.element("input");
    toolkit.set_attr(&h, "value", &props.text);
    toolkit.set_attr(&h, "placeholder", &props.placeholder);
    options(toolkit, h, &props.items);
    toolkit.listen(&h, listen::INPUT);
    h
}
fn options(toolkit: &mut Dom, h: DomHandle, items: &[String]) {
    toolkit.set_attr(
        &h,
        "data-day-completions",
        &serde_json::to_string(items).expect("string list"),
    );
}
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "`day_pieces::dom_renderer!` fixes the signature: handle and patch arrive by reference"
)]
fn update(toolkit: &mut Dom, h: &DomHandle, patch: &ComboPatch) {
    match patch {
        ComboPatch::SetText(text) => toolkit.set_attr(h, "value", text),
        ComboPatch::Items(items) => options(toolkit, *h, items),
    }
}
#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "`day_pieces::dom_renderer!` fixes the signature: handle and patch arrive by reference"
)]
fn measure(_toolkit: &mut Dom, _h: &DomHandle, proposal: Proposal) -> Size {
    Size::new(proposal.width.unwrap_or(180.0), 28.0)
}
day_pieces::dom_renderer!(crate::register_renderer, Dom,
    kind: KIND, props: ComboProps, patch: ComboPatch,
    make: make, update: update, measure: measure);
