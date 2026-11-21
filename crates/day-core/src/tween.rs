// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Interpolation for self-drawn content (docs/tween.md): what a canvas animation needs to say
//! what the in-between picture is.
//!
//! [`with_animation`](crate::with_animation) hands native widgets *intent* and lets each toolkit
//! animate on its own compositor. A canvas has no compositor to hand anything to: it re-records a
//! display list, so something has to decide, frame by frame, what the in-between picture is.
//! [`motion`](crate::motion) moves the values ([`Animated`](crate::motion::Animated),
//! [`Animation`](crate::motion::Animation)); this module says what lies between two of them:
//!
//! * [`Lerp`]: how a value moves between two states. Numbers, points, rects, tuples, arrays,
//!   vectors, and colors (blended in OKLab, so a heat map's mid-transition cells are the colors in
//!   between rather than a muddy detour through gray).
//! * [`stagger`]: one [`AnimSpec`] per item, each delayed a little more, for sequences (bars
//!   rising left to right).
//! * [`Pairs`]: the step before either when what changed is a *collection*. It matches the
//!   items of two states by key, gives the ones only one side has somewhere to come from or go
//!   to, and interpolates the lot, so bars that trade places slide and a new one grows out of
//!   its neighbor instead of the whole picture cross-fading.
//!
//! An `AnimSpec`'s timeline as a function of elapsed time is [`AnimSpec::sample`].

use std::collections::{HashMap, VecDeque};
use std::hash::Hash;

use day_spec::{Angle, AnimSpec, Color, Ink, Palette, Point, Rect, Size};

/// A value that can be interpolated: `a.lerp(&b, 0.0)` is `a`, `a.lerp(&b, 1.0)` is `b`.
///
/// `t` may leave `0..=1`: an under-damped spring overshoots, and a numeric value follows it past
/// the target and back. Implementations that cannot extrapolate (a color's alpha, a count) clamp
/// the parts that must stay in range.
pub trait Lerp: Clone {
    #[must_use]
    fn lerp(&self, to: &Self, t: f64) -> Self;
}

/// `a + (b - a) * t`, the scalar every other implementation is built from.
#[inline]
#[must_use]
pub fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

impl Lerp for f64 {
    fn lerp(&self, to: &Self, t: f64) -> Self {
        lerp(*self, *to, t)
    }
}

impl Lerp for f32 {
    fn lerp(&self, to: &Self, t: f64) -> Self {
        lerp(f64::from(*self), f64::from(*to), t) as f32
    }
}

impl Lerp for Point {
    fn lerp(&self, to: &Self, t: f64) -> Self {
        Point::new(lerp(self.x, to.x, t), lerp(self.y, to.y, t))
    }
}

impl Lerp for Size {
    fn lerp(&self, to: &Self, t: f64) -> Self {
        Size::new(
            lerp(self.width, to.width, t).max(0.0),
            lerp(self.height, to.height, t).max(0.0),
        )
    }
}

impl Lerp for Rect {
    fn lerp(&self, to: &Self, t: f64) -> Self {
        Rect {
            origin: self.origin.lerp(&to.origin, t),
            size: self.size.lerp(&to.size, t),
        }
    }
}

/// Colors blend in OKLab (Björn Ottosson's perceptual space), alpha linearly: the midpoint of blue
/// and yellow is a clean mid-lightness color rather than sRGB's dim gray, and a sequential scale's
/// in-between cells read as in-between values. The endpoints are exact.
impl Lerp for Color {
    fn lerp(&self, to: &Self, t: f64) -> Self {
        mix_oklab(*self, *to, t)
    }
}

/// An angle moves along the number line, not the short way round: from 350° to 10° is a turn of
/// 340° backwards, which is what a spinner counting down means. Normalize first for the short arc.
impl Lerp for Angle {
    fn lerp(&self, to: &Self, t: f64) -> Self {
        Angle::deg(lerp(self.degrees(), to.degrees(), t))
    }
}

/// Inks blend per appearance (docs/color.md "Inks in motion"). Two fixed colors or two light/dark
/// pairs blend each side in OKLab, so the in-between value still follows an appearance switch. A
/// [`SystemColor`](day_spec::SystemColor) role has no color of its own until the toolkit answers
/// it, so a blend that involves one resolves the role against [`Palette::both`] and blends the
/// pair.
/// The endpoints are exact: `t == 1` is `to` itself, role and all, so the settled value follows
/// the platform again.
impl Lerp for Ink {
    #[expect(
        clippy::float_cmp,
        reason = "the exact endpoints are the contract, so exact comparison is the point"
    )]
    fn lerp(&self, to: &Self, t: f64) -> Self {
        if t == 0.0 {
            return *self;
        }
        if t == 1.0 {
            return *to;
        }
        if let (Ink::Fixed(a), Ink::Fixed(b)) = (self, to) {
            return Ink::Fixed(mix_oklab(*a, *b, t));
        }
        let (light, dark) = Palette::both();
        let (a_light, a_dark) = self.pair(&light, &dark);
        let (b_light, b_dark) = to.pair(&light, &dark);
        Ink::adaptive(mix_oklab(a_light, b_light, t), mix_oklab(a_dark, b_dark, t))
    }
}

impl<A: Lerp, B: Lerp> Lerp for (A, B) {
    fn lerp(&self, to: &Self, t: f64) -> Self {
        (self.0.lerp(&to.0, t), self.1.lerp(&to.1, t))
    }
}

impl<A: Lerp, B: Lerp, C: Lerp> Lerp for (A, B, C) {
    fn lerp(&self, to: &Self, t: f64) -> Self {
        (
            self.0.lerp(&to.0, t),
            self.1.lerp(&to.1, t),
            self.2.lerp(&to.2, t),
        )
    }
}

impl<T: Lerp, const N: usize> Lerp for [T; N] {
    fn lerp(&self, to: &Self, t: f64) -> Self {
        std::array::from_fn(|i| self[i].lerp(&to[i], t))
    }
}

/// Element-wise over the common prefix; the target's extra elements (or the source's missing ones)
/// take the target's values, so a series that grew draws its new tail where it is headed. A caller
/// that wants entering elements to grow in matches them up first; `Vec` has no idea what they mean.
impl<T: Lerp> Lerp for Vec<T> {
    fn lerp(&self, to: &Self, t: f64) -> Self {
        to.iter()
            .enumerate()
            .map(|(i, b)| match self.get(i) {
                Some(a) => a.lerp(b, t),
                None => b.clone(),
            })
            .collect()
    }
}

/// Both present: interpolate. Otherwise there is nothing to move between, so the value switches at
/// the midpoint.
impl<T: Lerp> Lerp for Option<T> {
    fn lerp(&self, to: &Self, t: f64) -> Self {
        match (self, to) {
            (Some(a), Some(b)) => Some(a.lerp(b, t)),
            _ if t < 0.5 => self.clone(),
            _ => to.clone(),
        }
    }
}

// --- OKLab ------------------------------------------------------------------------------------

fn to_linear(c: f64) -> f64 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn to_srgb(c: f64) -> f64 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn oklab(c: Color) -> [f64; 3] {
    let (r, g, b) = (to_linear(c.r), to_linear(c.g), to_linear(c.b));
    let l = (0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b).cbrt();
    let m = (0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b).cbrt();
    let s = (0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b).cbrt();
    [
        0.210_454_255_3 * l + 0.793_617_785_0 * m - 0.004_072_046_8 * s,
        1.977_998_495_1 * l - 2.428_592_205_0 * m + 0.450_593_709_9 * s,
        0.025_904_037_1 * l + 0.782_771_766_2 * m - 0.808_675_766_0 * s,
    ]
}

fn from_oklab([l, a, b]: [f64; 3], alpha: f64) -> Color {
    let l_ = l + 0.396_337_777_4 * a + 0.215_803_757_3 * b;
    let m_ = l - 0.105_561_345_8 * a - 0.063_854_172_8 * b;
    let s_ = l - 0.089_484_177_5 * a - 1.291_485_548_0 * b;
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    let r = 4.076_741_662_1 * l3 - 3.307_711_591_3 * m3 + 0.230_969_929_2 * s3;
    let g = -1.268_438_004_6 * l3 + 2.609_757_401_1 * m3 - 0.341_319_396_5 * s3;
    let bl = -0.004_196_086_3 * l3 - 0.703_418_614_7 * m3 + 1.707_614_701_0 * s3;
    let ch = |v: f64| to_srgb(v.max(0.0)).clamp(0.0, 1.0);
    Color::rgba(ch(r), ch(g), ch(bl), alpha.clamp(0.0, 1.0))
}

/// Blend two colors in OKLab by `t`, alpha linearly. `t` outside `0..=1` extrapolates the
/// lightness and hue axes and clamps the result into gamut.
#[expect(
    clippy::float_cmp,
    reason = "exactly 0.0 and 1.0 select the endpoints unchanged"
)]
pub fn mix_oklab(a: Color, b: Color, t: f64) -> Color {
    // The endpoints are the endpoints, not a round trip through two color spaces.
    if t == 0.0 {
        return a;
    }
    if t == 1.0 {
        return b;
    }
    let (p, q) = (oklab(a), oklab(b));
    from_oklab(
        [
            lerp(p[0], q[0], t),
            lerp(p[1], q[1], t),
            lerp(p[2], q[2], t),
        ],
        lerp(a.a, b.a, t),
    )
}

// --- Staggering ------------------------------------------------------------------------------

/// `spec`, delayed for item `index` of `count` so the items start `spread_ms` apart in total:
/// the first starts on time, the last `spread_ms` later. Bars rising left to right, points popping
/// in along a line: a sequence is one spec per item, and this is the arithmetic.
pub fn stagger(spec: AnimSpec, index: usize, count: usize, spread_ms: u32) -> AnimSpec {
    let step = if count > 1 {
        f64::from(spread_ms) / (count - 1) as f64
    } else {
        0.0
    };
    AnimSpec {
        delay_ms: spec.delay_ms + (step * index as f64).round() as u32,
        ..spec
    }
}

// ---------------------------------------------------------------------------
// Pairs: matching two collections
// ---------------------------------------------------------------------------

/// Which side of a change an item of a [`Pairs`] is on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Change {
    /// In both states: it moves from where it was to where it is.
    Kept,
    /// Only in the new state: it arrives from a stand-in the caller chose.
    Entering,
    /// Only in the old state: it departs toward a stand-in the caller chose, and is gone once the
    /// transition ends.
    Leaving,
}

/// One item of a transition between two collections: where it starts, where it ends, and why.
#[derive(Clone, Debug, PartialEq)]
pub struct Pair<T> {
    pub from: T,
    pub to: T,
    pub change: Change,
}

impl<T: Lerp> Pair<T> {
    /// The item `t` of the way along.
    pub fn at(&self, t: f64) -> T {
        self.from.lerp(&self.to, t)
    }
}

/// Two states of a collection, matched item by item, ready to be sampled at any `t`.
///
/// Built once per change, sampled once per frame: matching is the part that costs something and
/// it does not depend on `t`. [`Pairs::by_key`] matches by an identity the caller derives; a
/// caller whose own rules already know what moved where (a sliding-tile game knows which tiles
/// merged) collects its [`Pair`]s directly (`Pairs: FromIterator`).
///
/// ```
/// use day_core::tween::{Change, Lerp, Pairs, lerp};
///
/// #[derive(Clone, Debug, PartialEq)]
/// struct Bar { name: &'static str, height: f64 }
///
/// impl Lerp for Bar {
///     fn lerp(&self, to: &Self, t: f64) -> Self {
///         Bar { name: to.name, height: lerp(self.height, to.height, t) }
///     }
/// }
///
/// let bar = |name, height| Bar { name, height };
/// // "b" shrinks, "c" grows out of nothing, "a" collapses away.
/// let before = [bar("a", 3.0), bar("b", 5.0)];
/// let after = [bar("b", 1.0), bar("c", 4.0)];
/// let pairs = Pairs::by_key(&before, &after, |b| b.name, |b, _| bar(b.name, 0.0));
/// assert_eq!(pairs.at(0.5), vec![bar("b", 3.0), bar("c", 2.0), bar("a", 1.5)]);
/// assert_eq!(pairs[2].change, Change::Leaving);
/// ```
#[derive(Clone, Debug, PartialEq)]
pub struct Pairs<T> {
    items: Vec<Pair<T>>,
}

impl<T> Default for Pairs<T> {
    fn default() -> Self {
        Pairs { items: Vec::new() }
    }
}

impl<T: Clone> Pairs<T> {
    /// Match `from` with `to` by `key`.
    ///
    /// * An item whose key is on both sides is [`Change::Kept`]. Keys may repeat: the *n*th item
    ///   with a key on one side matches the *n*th with that key on the other, so identical tokens
    ///   (three red gems in a column) pair up in order rather than all with the first.
    /// * An item only in `to` is [`Change::Entering`]; `absent(item, Change::Entering)` says where
    ///   it starts. An item only in `from` is [`Change::Leaving`]; `absent(item, Change::Leaving)`
    ///   says where it ends. That stand-in is the whole of what entering and leaving mean: grown
    ///   from a baseline, faded out, slid in from a neighbor. The caller decides; the match does
    ///   not.
    ///
    /// The result is in `to`'s order, then the leaving items in `from`'s order, so a renderer
    /// drawing in sequence draws the arriving picture's order, with what is departing on top.
    pub fn by_key<K: Eq + Hash>(
        from: &[T],
        to: &[T],
        mut key: impl FnMut(&T) -> K,
        mut absent: impl FnMut(&T, Change) -> T,
    ) -> Self {
        let mut waiting: HashMap<K, VecDeque<usize>> = HashMap::with_capacity(from.len());
        for (i, item) in from.iter().enumerate() {
            waiting.entry(key(item)).or_default().push_back(i);
        }
        let mut matched = vec![false; from.len()];
        let mut items = Vec::with_capacity(from.len().max(to.len()));
        for item in to {
            let found = waiting.get_mut(&key(item)).and_then(VecDeque::pop_front);
            items.push(match found {
                Some(i) => {
                    matched[i] = true;
                    Pair {
                        from: from[i].clone(),
                        to: item.clone(),
                        change: Change::Kept,
                    }
                }
                None => Pair {
                    from: absent(item, Change::Entering),
                    to: item.clone(),
                    change: Change::Entering,
                },
            });
        }
        for (item, _) in from.iter().zip(&matched).filter(|(_, m)| !**m) {
            items.push(Pair {
                from: item.clone(),
                to: absent(item, Change::Leaving),
                change: Change::Leaving,
            });
        }
        Pairs { items }
    }
}

impl<T> Pairs<T> {
    /// Every item `t` of the way along, in order, through `f`: for items whose in-between depends
    /// on more than the two ends (a chart mark needs both charts' scales to place itself).
    pub fn at_with<U>(&self, t: f64, mut f: impl FnMut(&Pair<T>, f64) -> U) -> Vec<U> {
        self.items.iter().map(|p| f(p, t)).collect()
    }

    /// Each item at its own progress: `progress(index, pair)` gives the `t` for that item, for
    /// sequences that do not move in lockstep (cards dealt one after another, see [`stagger`]).
    pub fn at_each<U>(
        &self,
        mut progress: impl FnMut(usize, &Pair<T>) -> f64,
        mut f: impl FnMut(&Pair<T>, f64) -> U,
    ) -> Vec<U> {
        self.items
            .iter()
            .enumerate()
            .map(|(i, p)| f(p, progress(i, p)))
            .collect()
    }

    /// The arriving state: every item that is not leaving, at its end. What to draw once the
    /// transition is over.
    pub fn settled(&self) -> impl Iterator<Item = &T> {
        self.items
            .iter()
            .filter(|p| p.change != Change::Leaving)
            .map(|p| &p.to)
    }

    /// How many items of each kind of [`Change`] there are: `(kept, entering, leaving)`.
    #[must_use]
    pub fn counts(&self) -> (usize, usize, usize) {
        self.items
            .iter()
            .fold((0, 0, 0), |(k, e, l), p| match p.change {
                Change::Kept => (k + 1, e, l),
                Change::Entering => (k, e + 1, l),
                Change::Leaving => (k, e, l + 1),
            })
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Pair<T>> {
        self.items.iter()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.items.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl<T: Lerp> Pairs<T> {
    /// Every item `t` of the way along, in order. Leaving items are included until the caller
    /// stops sampling: at `t = 1` they sit at their stand-ins.
    #[must_use]
    pub fn at(&self, t: f64) -> Vec<T> {
        self.items.iter().map(|p| p.at(t)).collect()
    }
}

impl<T> std::ops::Index<usize> for Pairs<T> {
    type Output = Pair<T>;
    fn index(&self, i: usize) -> &Pair<T> {
        &self.items[i]
    }
}

impl<T> FromIterator<Pair<T>> for Pairs<T> {
    fn from_iter<I: IntoIterator<Item = Pair<T>>>(iter: I) -> Self {
        Pairs {
            items: iter.into_iter().collect(),
        }
    }
}

impl<T> IntoIterator for Pairs<T> {
    type Item = Pair<T>;
    type IntoIter = std::vec::IntoIter<Pair<T>>;
    fn into_iter(self) -> Self::IntoIter {
        self.items.into_iter()
    }
}

impl<'a, T> IntoIterator for &'a Pairs<T> {
    type Item = &'a Pair<T>;
    type IntoIter = std::slice::Iter<'a, Pair<T>>;
    fn into_iter(self) -> Self::IntoIter {
        self.items.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn stagger_spreads_delays_across_the_items() {
        let s = AnimSpec::linear(300);
        assert_eq!(stagger(s, 0, 5, 400).delay_ms, 0);
        assert_eq!(stagger(s, 2, 5, 400).delay_ms, 200);
        assert_eq!(stagger(s, 4, 5, 400).delay_ms, 400);
        assert_eq!(stagger(s, 0, 1, 400).delay_ms, 0);
    }

    #[test]
    fn values_interpolate_and_colors_keep_their_endpoints() {
        assert!(near(10.0.lerp(&20.0, 0.25), 12.5));
        let r = Rect::new(0.0, 0.0, 10.0, 10.0).lerp(&Rect::new(10.0, 10.0, 30.0, 10.0), 0.5);
        assert_eq!(r, Rect::new(5.0, 5.0, 20.0, 10.0));
        let (blue, yellow) = (Color::rgb(0.0, 0.0, 1.0), Color::rgb(1.0, 1.0, 0.0));
        let a = blue.lerp(&yellow, 0.0);
        let b = blue.lerp(&yellow, 1.0);
        assert_eq!(a, blue);
        assert_eq!(b, yellow);
        // The OKLab midpoint of blue and yellow is lighter than sRGB's gray (0.5, 0.5, 0.5).
        let mid = blue.lerp(&yellow, 0.5);
        assert!(mid.r + mid.g + mid.b > 1.6, "{mid:?}");
        assert_eq!(
            vec![0.0, 1.0].lerp(&vec![2.0, 3.0, 9.0], 0.5),
            vec![1.0, 2.0, 9.0]
        );
        assert_eq!(Some(1.0).lerp(&None, 0.4), Some(1.0));
        assert_eq!(Some(1.0).lerp(&None, 0.6), None);
    }

    // --- Pairs -----------------------------------------------------------------------------

    /// A keyed value, the shape most callers pair: an identity and something that moves.
    #[derive(Clone, Debug, PartialEq)]
    struct Item {
        key: &'static str,
        at: f64,
    }

    impl Lerp for Item {
        fn lerp(&self, to: &Self, t: f64) -> Self {
            Item {
                key: to.key,
                at: lerp(self.at, to.at, t),
            }
        }
    }

    fn item(key: &'static str, at: f64) -> Item {
        Item { key, at }
    }

    /// Stand-ins at zero, the way a bar grows from its baseline.
    fn from_zero(i: &Item, _: Change) -> Item {
        item(i.key, 0.0)
    }

    fn by_key(from: &[Item], to: &[Item]) -> Pairs<Item> {
        Pairs::by_key(from, to, |i| i.key, from_zero)
    }

    fn changes(p: &Pairs<Item>) -> Vec<(&'static str, Change)> {
        p.iter().map(|p| (p.to.key, p.change)).collect()
    }

    #[test]
    fn items_are_kept_entering_or_leaving_in_arriving_order_then_leaving() {
        let p = by_key(
            &[item("a", 1.0), item("b", 2.0), item("c", 3.0)],
            &[item("c", 30.0), item("d", 40.0), item("a", 10.0)],
        );
        assert_eq!(
            changes(&p),
            vec![
                ("c", Change::Kept),
                ("d", Change::Entering),
                ("a", Change::Kept),
                ("b", Change::Leaving),
            ]
        );
        assert_eq!(p.counts(), (2, 1, 1));
        // A kept item starts where it was, an entering one at its stand-in, a leaving one where
        // it was and ends at its stand-in.
        assert_eq!((p[0].from.at, p[0].to.at), (3.0, 30.0));
        assert_eq!((p[1].from.at, p[1].to.at), (0.0, 40.0));
        assert_eq!((p[2].from.at, p[2].to.at), (1.0, 10.0));
        assert_eq!((p[3].from.at, p[3].to.at), (2.0, 0.0));
    }

    #[test]
    fn the_stand_in_is_asked_for_with_the_item_and_its_side() {
        let mut asked = Vec::new();
        let p = Pairs::by_key(
            &[item("gone", 5.0), item("stays", 1.0)],
            &[item("stays", 2.0), item("new", 7.0)],
            |i| i.key,
            |i, change| {
                asked.push((i.key, i.at, change));
                // Entering items come from below their target, leaving ones go above.
                match change {
                    Change::Entering => item(i.key, i.at - 100.0),
                    Change::Leaving => item(i.key, i.at + 100.0),
                    Change::Kept => unreachable!("a kept item has both ends"),
                }
            },
        );
        assert_eq!(
            asked,
            vec![
                ("new", 7.0, Change::Entering),
                ("gone", 5.0, Change::Leaving)
            ]
        );
        assert_eq!(p[1].from.at, -93.0);
        assert_eq!(p[2].to.at, 105.0);
    }

    #[test]
    fn repeated_keys_pair_in_order_and_the_surplus_enters_or_leaves() {
        // Three red gems become two: the first two pair with the first two, the third leaves.
        let p = by_key(
            &[item("red", 1.0), item("red", 2.0), item("red", 3.0)],
            &[item("red", 10.0), item("red", 20.0)],
        );
        assert_eq!(p.counts(), (2, 0, 1));
        assert_eq!((p[0].from.at, p[1].from.at), (1.0, 2.0));
        assert_eq!((p[2].from.at, p[2].change), (3.0, Change::Leaving));
        // And the other way: the extra one enters.
        let p = by_key(&[item("red", 1.0)], &[item("red", 10.0), item("red", 20.0)]);
        assert_eq!(p.counts(), (1, 1, 0));
        assert_eq!((p[0].from.at, p[1].change), (1.0, Change::Entering));
    }

    #[test]
    fn an_empty_side_makes_everything_enter_or_leave() {
        let some = [item("a", 1.0), item("b", 2.0)];
        let arriving = by_key(&[], &some);
        assert_eq!(arriving.counts(), (0, 2, 0));
        assert_eq!(arriving.at(0.0), vec![item("a", 0.0), item("b", 0.0)]);
        let departing = by_key(&some, &[]);
        assert_eq!(departing.counts(), (0, 0, 2));
        assert_eq!(departing.at(1.0), vec![item("a", 0.0), item("b", 0.0)]);
        assert!(by_key(&[], &[]).is_empty());
        assert!(Pairs::<Item>::default().is_empty());
    }

    #[test]
    fn a_permutation_keeps_everything_and_follows_the_new_order() {
        let from = [item("a", 0.0), item("b", 1.0), item("c", 2.0)];
        let to = [item("c", 0.0), item("a", 1.0), item("b", 2.0)];
        let p = by_key(&from, &to);
        assert_eq!(p.counts(), (3, 0, 0));
        assert_eq!(
            p.at(0.0),
            vec![item("c", 2.0), item("a", 0.0), item("b", 1.0)]
        );
        assert_eq!(p.at(1.0), to.to_vec());
    }

    #[test]
    fn sampling_runs_from_the_old_state_to_the_new_one_and_past_it() {
        let p = by_key(&[item("a", 0.0), item("b", 8.0)], &[item("a", 10.0)]);
        assert_eq!(p.at(0.0), vec![item("a", 0.0), item("b", 8.0)]);
        assert_eq!(p.at(0.25), vec![item("a", 2.5), item("b", 6.0)]);
        // At the end the leaving item sits at its stand-in; the caller drops it when done.
        assert_eq!(p.at(1.0), vec![item("a", 10.0), item("b", 0.0)]);
        // A spring's overshoot carries the values past their targets.
        assert_eq!(p.at(1.1)[0].at, 11.0);
        assert_eq!(
            p.settled().cloned().collect::<Vec<_>>(),
            vec![item("a", 10.0)]
        );
    }

    #[test]
    fn at_with_hands_each_pair_and_t_to_the_caller() {
        let p = by_key(&[item("a", 0.0)], &[item("a", 4.0), item("b", 2.0)]);
        let got = p.at_with(0.5, |pair, t| (pair.change, pair.at(t).at));
        assert_eq!(got, vec![(Change::Kept, 2.0), (Change::Entering, 1.0)]);
    }

    #[test]
    fn at_each_gives_every_item_its_own_progress() {
        // Dealt one after another: item i starts i quarters later.
        let to: Vec<Item> = ["a", "b", "c"].iter().map(|k| item(k, 10.0)).collect();
        let p = by_key(&[], &to);
        let elapsed = 0.5;
        let got = p.at_each(
            |i, _| ((elapsed - i as f64 * 0.25) / 0.5).clamp(0.0, 1.0),
            |pair, t| pair.at(t).at,
        );
        assert_eq!(got, vec![10.0, 5.0, 0.0]);
    }

    #[test]
    fn pairs_collected_directly_may_share_a_destination() {
        // Two tiles merging into one cell: the rules know it, and a key match could not say it.
        let p: Pairs<Point> = [
            (Point::new(0.0, 0.0), Point::new(3.0, 0.0)),
            (Point::new(1.0, 0.0), Point::new(3.0, 0.0)),
        ]
        .into_iter()
        .map(|(from, to)| Pair {
            from,
            to,
            change: Change::Kept,
        })
        .collect();
        assert_eq!(p.len(), 2);
        let mid = p.at(0.5);
        assert_eq!(mid, vec![Point::new(1.5, 0.0), Point::new(2.0, 0.0)]);
        assert_eq!(p.at(1.0), vec![Point::new(3.0, 0.0); 2]);
    }

    #[test]
    fn pairs_of_library_types_interpolate_with_their_own_lerp() {
        let (red, blue) = (Color::rgb(1.0, 0.0, 0.0), Color::rgb(0.0, 0.0, 1.0));
        let from = [(1u8, Rect::new(0.0, 0.0, 10.0, 10.0), red)];
        let to = [(1u8, Rect::new(20.0, 0.0, 10.0, 30.0), blue)];
        let p = Pairs::by_key(
            &from.map(|(_, r, c)| (r, c)),
            &to.map(|(_, r, c)| (r, c)),
            |_| 1u8,
            |x, _| *x,
        );
        let (rect, color) = p.at(0.5)[0];
        assert_eq!(rect, Rect::new(10.0, 0.0, 10.0, 20.0));
        assert!(color != red && color != blue);
        assert_eq!(p.at(1.0)[0].1, blue);
    }

    #[test]
    fn keys_are_derived_once_per_item() {
        let calls = Cell::new(0);
        let from = [item("a", 0.0), item("b", 0.0), item("c", 0.0)];
        let to = [item("b", 0.0), item("d", 0.0)];
        let _ = Pairs::by_key(
            &from,
            &to,
            |i| {
                calls.set(calls.get() + 1);
                i.key
            },
            from_zero,
        );
        assert_eq!(calls.get(), from.len() + to.len());
    }

    #[test]
    fn iteration_and_indexing_see_the_same_pairs() {
        let p = by_key(&[item("a", 1.0)], &[item("a", 2.0), item("b", 3.0)]);
        let by_ref: Vec<_> = (&p).into_iter().map(|p| p.to.key).collect();
        let by_iter: Vec<_> = p.iter().map(|p| p.to.key).collect();
        assert_eq!(by_ref, by_iter);
        assert_eq!(p[1].to.key, "b");
        let owned: Vec<Pair<Item>> = p.clone().into_iter().collect();
        assert_eq!(owned.len(), p.len());
        assert_eq!(owned[0], p[0]);
    }

    /// For random collections with repeated keys, the match accounts for every item exactly
    /// once: each arriving item is the end of exactly one non-leaving pair (in its order), each
    /// departing item the start of exactly one kept or leaving pair, and the number kept is the
    /// size of the keys' multiset intersection.
    #[test]
    fn every_item_is_accounted_for_exactly_once() {
        const KEYS: [&str; 5] = ["a", "b", "c", "d", "e"];
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = move |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        for round in 0..200 {
            let len_a = next(9);
            let len_b = next(9);
            // Distinct positions so each item can be recognized after matching.
            let from: Vec<Item> = (0..len_a)
                .map(|i| item(KEYS[next(KEYS.len())], i as f64))
                .collect();
            let to: Vec<Item> = (0..len_b)
                .map(|i| item(KEYS[next(KEYS.len())], 100.0 + i as f64))
                .collect();
            let p = Pairs::by_key(
                &from,
                &to,
                |i| i.key,
                |i, c| match c {
                    Change::Entering => item(i.key, -1.0),
                    _ => item(i.key, -2.0),
                },
            );
            let arriving: Vec<&Item> = p.settled().collect();
            assert_eq!(arriving, to.iter().collect::<Vec<_>>(), "round {round}");
            let mut starts: Vec<f64> = p
                .iter()
                .filter(|p| p.change != Change::Entering)
                .map(|p| p.from.at)
                .collect();
            starts.sort_by(f64::total_cmp);
            let all: Vec<f64> = from.iter().map(|i| i.at).collect();
            assert_eq!(starts, all, "round {round}");
            let kept = KEYS
                .iter()
                .map(|k| {
                    let a = from.iter().filter(|i| i.key == *k).count();
                    let b = to.iter().filter(|i| i.key == *k).count();
                    a.min(b)
                })
                .sum::<usize>();
            let (k, e, l) = p.counts();
            assert_eq!(
                (k, e, l),
                (kept, len_b - kept, len_a - kept),
                "round {round}"
            );
            for pair in &p {
                match pair.change {
                    Change::Kept => assert_eq!(pair.from.key, pair.to.key),
                    Change::Entering => assert_eq!(pair.from.at, -1.0),
                    Change::Leaving => assert_eq!(pair.to.at, -2.0),
                }
            }
        }
    }
}
