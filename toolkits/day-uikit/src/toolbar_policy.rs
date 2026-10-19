// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

use day_spec::{ToolbarColumn as C, ToolbarItem, ToolbarPlacement as P};

/// A collapsed stack merges commands; an expanded split honors their declaration site.
pub(super) fn belongs_to_column(owner: C, column: Option<C>) -> bool {
    column.is_none_or(|column| owner == column || (column == C::Detail && owner == C::Window))
}

/// Plain navigation bars (iOS 15 and shared tab bars) cannot overflow automatically.
/// Keep at most three controls, including More, leaving room for a title and native Back.
/// Leading navigation actions precede Primary actions; the menu preserves declaration order.
pub(super) fn legacy_visible(items: &[ToolbarItem]) -> Vec<usize> {
    let controls: Vec<_> = items
        .iter()
        .enumerate()
        .filter(|(_, item)| {
            matches!(
                item.placement,
                P::Navigation | P::Primary | P::Automatic | P::Secondary
            )
        })
        .collect();
    let overflow = controls.len() > 3
        || controls
            .iter()
            .any(|(_, item)| item.placement == P::Secondary);
    let mut priority = controls;
    priority.sort_by_key(|(_, item)| match item.placement {
        P::Navigation => 0,
        P::Primary => 1,
        _ => 2,
    });
    let mut visible: Vec<_> = priority
        .into_iter()
        .filter(|(_, item)| item.placement != P::Secondary)
        .take(if overflow { 2 } else { 3 })
        .map(|(index, _)| index)
        .collect();
    // Title and bottom-bar items use their own slots, never the navigation overflow.
    visible.extend(
        items
            .iter()
            .enumerate()
            .filter(|(_, item)| matches!(item.placement, P::Principal | P::Bottom))
            .map(|(index, _)| index),
    );
    visible
}

#[cfg(test)]
mod tests {
    use super::*;
    use day_spec::{LabelStyle, ToolbarItemKind};

    fn item(placement: P) -> ToolbarItem {
        ToolbarItem {
            id: "fixture".into(),
            label: "Fixture".into(),
            kind: ToolbarItemKind::Button,
            tooltip: None,
            icon: None,
            enabled: true,
            action: 0,
            placement,
            label_style: LabelStyle::Automatic,
            prominent: false,
            column: C::Detail,
        }
    }

    #[test]
    fn split_commands_have_exactly_one_owner_and_collapse_merges() {
        for owner in [C::Sidebar, C::List, C::Detail, C::Window] {
            assert!(belongs_to_column(owner, None));
            let columns: Vec<_> = [C::Sidebar, C::List, C::Detail]
                .into_iter()
                .filter(|column| belongs_to_column(owner, Some(*column)))
                .collect();
            assert_eq!(
                columns,
                vec![if owner == C::Window { C::Detail } else { owner }]
            );
        }
    }

    #[test]
    fn crowded_reader_keeps_primary_and_overflows_all_other_commands() {
        let items: Vec<_> = [
            P::Automatic,
            P::Automatic,
            P::Primary,
            P::Automatic,
            P::Secondary,
            P::Primary,
        ]
        .into_iter()
        .map(item)
        .collect();
        assert_eq!(legacy_visible(&items), vec![2, 5]);
    }

    #[test]
    fn list_controls_remain_visible_and_navigation_never_loses_actions() {
        let mut items = vec![item(P::Navigation), item(P::Navigation)];
        assert_eq!(legacy_visible(&items), vec![0, 1]);
        items.extend([
            item(P::Primary),
            item(P::Secondary),
            item(P::Principal),
            item(P::Bottom),
        ]);
        assert_eq!(legacy_visible(&items), vec![0, 1, 4, 5]);
        // Third and fourth controls are still represented by More, not discarded.
        let visible = legacy_visible(&items);
        assert_eq!(
            (0..items.len())
                .filter(|i| !visible.contains(i))
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
    }
}
